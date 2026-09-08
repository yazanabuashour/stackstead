use std::{collections::BTreeMap, io};

use crate::{compose, lifecycle, manifest::ComponentStatus, output};

pub(super) fn next_actions(stackstead_id: &str, runtime_status: ComponentStatus) -> [String; 2] {
    let runtime = match runtime_status {
        ComponentStatus::Running => format!("stackstead logs {stackstead_id} --tail 200"),
        ComponentStatus::Stopped => format!("stackstead up {stackstead_id}"),
        ComponentStatus::Created
        | ComponentStatus::Ready
        | ComponentStatus::Reachable
        | ComponentStatus::Unreachable
        | ComponentStatus::Failed
        | ComponentStatus::Unknown => "stackstead doctor".into(),
    };
    [
        runtime,
        format!("stackstead context {stackstead_id} --print"),
    ]
}

pub(super) fn print_runtime(inspection: &lifecycle::InspectOutput) {
    let runtime = &inspection.live.runtime;
    println!("Runtime activity: {}", runtime.activity());
    println!("Runtime readiness: {}", runtime.readiness.status);
    for service in &runtime.readiness.required {
        let expected = service
            .expected_instances
            .map_or_else(|| "unknown".into(), |count| count.to_string());
        println!(
            "  {} ({}): {} instances satisfied={}/{} observed={}",
            service.service,
            service.role.as_str(),
            service.status,
            service.satisfied_instances,
            expected,
            service.observed_containers
        );
    }
    for issue in &runtime.readiness.issues {
        println!("  - {issue}");
    }
    println!("Live runtime:  {}", runtime.status());
    println!(
        "Effective:     runtime={} ({}) health={} ({})",
        inspection.effective.runtime.status,
        inspection.effective.runtime.basis,
        inspection.effective.health.status,
        inspection.effective.health.basis
    );
    println!("Services:");
    match &runtime.services {
        None => println!("  unknown"),
        Some(services) if services.is_empty() => println!("  none"),
        Some(services) => {
            for service in services {
                let health = service.health.as_deref().unwrap_or_else(|| {
                    if service.healthcheck_enabled == Some(false) {
                        "not configured"
                    } else {
                        "unknown"
                    }
                });
                let name = if service.service.is_empty() {
                    "unattributed"
                } else {
                    &service.service
                };
                println!(
                    "  {name}/{}: {} health={health}",
                    service.container,
                    service.status()
                );
            }
        }
    }
}

pub(super) fn print_up_timings(timings: &lifecycle::UpTimings) {
    println!("\nTimings:");
    print_timing("Dependencies", timings.dependencies);
    print_timing("Runtime start", timings.runtime);
    for (label, duration) in [
        ("DB readiness", timings.database),
        ("Seed", timings.seed),
        ("Hooks", timings.hooks),
        ("Health checks", timings.health),
    ] {
        if let Some(duration) = duration {
            print_timing(label, duration);
        }
    }
    print_timing("Total", timings.total);
}

fn print_timing(label: &str, duration: std::time::Duration) {
    let elapsed = if duration.as_millis() == 0 {
        "<1ms".into()
    } else if duration.as_secs() == 0 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    };
    println!("  {label:<14} {elapsed:>8}");
}

pub(super) fn print_urls(urls: &BTreeMap<String, String>) {
    if !urls.is_empty() {
        println!("URLs:");
        for (service, url) in urls {
            println!("  {service:<14} {url}");
        }
    }
}

pub(super) fn print_compose_plan(plan: &compose::ComposePlan) {
    println!("Compose: {}", plan.file.display());
    if plan.ports.is_empty() {
        println!("No published service ports detected.");
    } else {
        println!("Detected isolation contract:");
        for port in &plan.ports {
            println!(
                "  {:<16} container {:<5} env {:<24} mapping {}",
                port.name, port.container_port, port.env, port.replacement
            );
        }
    }
    let fixed = plan
        .ports
        .iter()
        .filter_map(|port| port.current_host_port.map(|host| (port, host)))
        .collect::<Vec<_>>();
    if !fixed.is_empty() {
        println!("Required Compose edits before `stackstead up`:");
        for (port, host_port) in fixed {
            println!(
                "  {}: replace fixed host port {} with `{}`",
                port.service, host_port, port.replacement
            );
        }
        println!(
            "Run `stackstead compose apply --yes` to make these narrow edits, then review the Git diff."
        );
    }
    for warning in &plan.warnings {
        println!("Warning: {warning}");
    }
}

pub(super) fn print_json<T: output::CliOutput>(value: &T) -> anyhow::Result<()> {
    serde_json::to_writer_pretty(io::stdout().lock(), value)?;
    println!();
    Ok(())
}
