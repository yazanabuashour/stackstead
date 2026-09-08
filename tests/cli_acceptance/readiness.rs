use super::*;

#[path = "readiness_fixture.rs"]
pub(super) mod fixture;
#[path = "readiness_resolution.rs"]
mod resolution;
#[path = "readiness_scenario.rs"]
mod scenario;
#[path = "readiness_snapshots.rs"]
mod snapshots;
#[path = "readiness_startup.rs"]
mod startup;

use fixture::{HASH, row};
use scenario::ReadinessFixture;

#[test]
fn ps_and_inspect_share_required_instance_evidence_without_inferring_jobs() -> anyhow::Result<()> {
    let fixture =
        ReadinessFixture::new(serde_json::json!({"worker": "long-running", "migrate": "job"}))?;
    fixture.up()?;
    assert_shared_ready_reading(&fixture)?;
    for (case, expected) in [
        ("missing instance", "not_ready"),
        ("long-running exited zero", "not_ready"),
        ("failed job", "not_ready"),
        ("missing job", "not_ready"),
        ("image healthcheck unhealthy", "not_ready"),
        ("healthcheck starting", "not_ready"),
        ("missing health status", "unknown"),
        ("unknown healthcheck", "unknown"),
        ("missing ordinal", "unknown"),
        ("unknown oneoff", "unknown"),
        ("missing hash", "unknown"),
        ("unknown state", "unknown"),
        ("extra instance", "unknown"),
        ("duplicate ordinal", "unknown"),
        ("unattributed container", "unknown"),
        ("known failure with unknown evidence", "not_ready"),
        ("oneoff cannot replace required instance", "not_ready"),
        ("optional unhealthy", "ready"),
        ("excluded oneoff", "ready"),
        ("image healthcheck healthy", "ready"),
    ] {
        fixture.docker.rows(&changed_rows(&fixture, case)?)?;
        let inspected = fixture.readings()?;
        assert_eq!(
            inspected["live"]["readiness"]["status"], expected,
            "{case}: {inspected}"
        );
        assert!(inspected["live"]["services"].is_array(), "{case}");
        if expected != "ready" {
            assert!(
                !inspected["live"]["readiness"]["issues"]
                    .as_array()
                    .test()?
                    .is_empty(),
                "{case}"
            );
        }
    }
    Ok(())
}

fn assert_shared_ready_reading(fixture: &ReadinessFixture) -> anyhow::Result<()> {
    let ready = fixture.readings()?;
    assert_eq!(ready["live"]["readiness"]["status"], "ready");
    assert_eq!(ready["live"]["runtime"]["activity"], "active");
    assert_eq!(ready["live"]["runtime"]["running"], true);
    let required = ready["live"]["readiness"]["required"].as_array().test()?;
    let worker = required
        .iter()
        .find(|item| item["service"] == "worker")
        .test()?;
    assert_eq!(worker["role"], "long-running");
    assert_eq!(worker["expected_instances"], 2);
    assert_eq!(worker["observed_containers"], 2);
    assert_eq!(worker["satisfied_instances"], 2);
    assert_eq!(worker["status"], "ready");
    assert_eq!(worker["issues"], serde_json::json!([]));
    let services = ready["live"]["services"].as_array().test()?;
    let job = services
        .iter()
        .find(|item| item["service"] == "migrate")
        .test()?;
    assert_eq!(job["status"], "exited (0)");
    assert_eq!(job["state"], "exited");
    assert_eq!(job["exit_code"], 0);
    let optional = services
        .iter()
        .find(|item| item["service"] == "optional")
        .test()?;
    assert_eq!(optional["exit_code"], 7);
    assert_eq!(optional["status"], "exited (7)");
    let workers = services
        .iter()
        .filter(|item| item["service"] == "worker")
        .collect::<Vec<_>>();
    assert_eq!(
        workers
            .iter()
            .map(|item| item["container_number"].as_u64().test())
            .collect::<anyhow::Result<Vec<_>>>()?,
        vec![2, 3]
    );
    for worker in workers {
        assert_eq!(worker["config_hash"], HASH);
        assert_eq!(worker["oneoff"], false);
        assert_eq!(worker["healthcheck_enabled"], false);
        assert!(worker["health"].is_null());
        assert!(worker["id"].as_str().is_some_and(|id| id.len() == 64));
    }
    let human = fixture
        .docker
        .command(&fixture.manifest)
        .args(["inspect", &fixture.manifest.stackstead_id])
        .assert()
        .success();
    let human = output_text(&human.get_output().stdout)?;
    assert!(human.contains("exited (0)"));
    assert!(human.contains("exited (7)"));
    assert!(!human.contains("completed (0)"));
    fixture.docker.assert_supported()
}

fn changed_rows(fixture: &ReadinessFixture, case: &str) -> anyhow::Result<Vec<Value>> {
    let mut rows = fixture.rows.clone();
    match case {
        "missing instance" => {
            rows.remove(0);
        }
        "long-running exited zero" => rows[0]["state"] = "exited".into(),
        "failed job" => rows[2]["exit_code"] = 7.into(),
        "missing job" => {
            rows.remove(2);
        }
        "image healthcheck unhealthy"
        | "healthcheck starting"
        | "missing health status"
        | "image healthcheck healthy" => {
            rows[0]["healthcheck_enabled"] = true.into();
            rows[0]["health"] = match case {
                "image healthcheck unhealthy" => "unhealthy".into(),
                "healthcheck starting" => "starting".into(),
                "image healthcheck healthy" => "healthy".into(),
                _ => Value::Null,
            };
        }
        "unknown healthcheck" => rows[0]["healthcheck_enabled"] = Value::Null,
        "missing ordinal" => rows[0]["container_number"] = Value::Null,
        "unknown oneoff" => rows[0]["oneoff"] = Value::Null,
        "missing hash" => rows[0]["config_hash"] = Value::Null,
        "unknown state" => rows[0]["state"] = "future-state".into(),
        "extra instance" => rows.push(row(&fixture.manifest, "worker", 4, 6, "running", 0)),
        "duplicate ordinal" => {
            let mut duplicate = row(&fixture.manifest, "worker", 2, 6, "running", 0);
            duplicate["container"] =
                format!("/{}-worker-old-fixture", fixture.manifest.compose_project).into();
            rows.push(duplicate);
        }
        "unattributed container" => rows[3]["service"] = Value::Null,
        "known failure with unknown evidence" => {
            rows[0]["healthcheck_enabled"] = Value::Null;
            rows[2]["exit_code"] = 7.into();
        }
        "oneoff cannot replace required instance" => rows[0]["oneoff"] = "True".into(),
        "optional unhealthy" => {
            rows[3]["state"] = "running".into();
            rows[3]["healthcheck_enabled"] = true.into();
            rows[3]["health"] = "unhealthy".into();
        }
        "excluded oneoff" => {
            let mut oneoff = row(&fixture.manifest, "worker", 2, 6, "exited", 9);
            oneoff["container"] =
                format!("/{}-worker-run-fixture", fixture.manifest.compose_project).into();
            oneoff["oneoff"] = "True".into();
            rows.push(oneoff);
        }
        _ => anyhow::bail!("unknown readiness case {case}"),
    }
    Ok(rows)
}
