use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::is_sha256;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    LongRunning,
    Job,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LongRunning => "long-running",
            Self::Job => "job",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "configuration", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Contract {
    Unconfigured {},
    Declared {
        required: BTreeMap<String, Role>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolved: Option<Requirements>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    pub profiles: Option<String>,
    pub model_hash: String,
    pub services: BTreeMap<String, ServiceRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequirement {
    pub replicas: u64,
    pub config_hash: String,
}

impl Contract {
    pub const fn required(&self) -> Option<&BTreeMap<String, Role>> {
        match self {
            Self::Unconfigured {} => None,
            Self::Declared { required, .. } => Some(required),
        }
    }

    pub const fn resolved(&self) -> Option<&Requirements> {
        match self {
            Self::Unconfigured {} => None,
            Self::Declared { resolved, .. } => resolved.as_ref(),
        }
    }

    pub fn invalidate(&mut self) {
        if let Self::Declared { resolved, .. } = self {
            *resolved = None;
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let Self::Declared { required, resolved } = self else {
            return Ok(());
        };
        if required.is_empty() {
            anyhow::bail!("readiness.required must contain at least one service");
        }
        let Some(resolved) = resolved else {
            return Ok(());
        };
        if required.keys().ne(resolved.services.keys()) {
            anyhow::bail!("readiness resolved service keys must exactly match required services");
        }
        validate_hash(&resolved.model_hash, "readiness resolved model_hash")?;
        for service in resolved.services.values() {
            if service.replicas == 0 {
                anyhow::bail!("readiness resolved replicas must be greater than zero");
            }
            validate_hash(
                &service.config_hash,
                "readiness resolved service config_hash",
            )?;
        }
        Ok(())
    }
}

fn validate_hash(value: &str, field: &str) -> anyhow::Result<()> {
    if !is_sha256(value) {
        anyhow::bail!("{field} must contain exactly 64 lowercase hexadecimal characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

    fn declared() -> Contract {
        Contract::Declared {
            required: BTreeMap::from([("Worker.api_1".into(), Role::Job)]),
            resolved: Some(Requirements {
                profiles: Some("jobs, background".into()),
                model_hash: "ab".repeat(32),
                services: BTreeMap::from([(
                    "Worker.api_1".into(),
                    ServiceRequirement {
                        replicas: u64::MAX,
                        config_hash: "01".repeat(32),
                    },
                )]),
            }),
        }
    }

    #[test]
    fn round_trip_and_invalidation_preserve_explicit_intent() -> anyhow::Result<()> {
        let mut contract = declared();
        contract.validate().test()?;
        let value = serde_json::to_value(&contract).test()?;
        assert_eq!(value["configuration"], "declared");
        assert_eq!(value["required"]["Worker.api_1"], Role::Job.as_str());
        assert_eq!(Role::LongRunning.as_str(), "long-running");
        assert_eq!(serde_json::from_value::<Contract>(value).test()?, contract);
        let required = contract.required().cloned();
        contract.invalidate();
        contract.validate().test()?;
        assert_eq!(contract.required(), required.as_ref());
        assert!(contract.resolved().is_none());
        let value = serde_json::to_value(&contract).test()?;
        assert!(value.get("resolved").is_none());
        assert_eq!(serde_json::from_value::<Contract>(value).test()?, contract);
        let mut unconfigured = Contract::Unconfigured {};
        unconfigured.invalidate();
        unconfigured.validate().test()?;
        assert!(unconfigured.required().is_none());
        assert!(unconfigured.resolved().is_none());
        assert_eq!(
            serde_json::to_value(unconfigured).test()?,
            serde_json::json!({"configuration": "unconfigured"})
        );
        Ok(())
    }

    #[test]
    fn rejects_corrupt_resolved_contracts() -> anyhow::Result<()> {
        let original = serde_json::to_value(declared()).test()?;
        for (pointer, replacement) in [
            ("/required", serde_json::json!({})),
            ("/resolved/services", serde_json::json!({})),
            (
                "/resolved/services/Worker.api_1/replicas",
                serde_json::json!(0),
            ),
            ("/resolved/model_hash", serde_json::json!("AB".repeat(32))),
            ("/resolved/model_hash", serde_json::json!("a".repeat(63))),
            ("/resolved/model_hash", serde_json::json!("a".repeat(65))),
            ("/resolved/model_hash", serde_json::json!("g".repeat(64))),
            (
                "/resolved/services/Worker.api_1/config_hash",
                serde_json::json!("bad"),
            ),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).test()? = replacement;
            serde_json::from_value::<Contract>(value)
                .test()?
                .validate()
                .test_err()?;
        }
        let mut value = original;
        let services = value["resolved"]["services"].as_object_mut().test()?;
        let service = services.remove("Worker.api_1").test()?;
        services.insert("different".into(), service);
        serde_json::from_value::<Contract>(value)
            .test()?
            .validate()
            .test_err()?;
        Ok(())
    }

    #[test]
    fn rejects_unknown_fields_roles_and_invalid_shapes() -> anyhow::Result<()> {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"configuration": "ready"}),
            serde_json::json!({"configuration": "unconfigured", "required": {}}),
            serde_json::json!({"configuration": "unconfigured", "resolved": null}),
            serde_json::json!({"configuration": "declared"}),
            serde_json::json!({"configuration": "declared", "required": {"web": "running"}}),
        ] {
            serde_json::from_value::<Contract>(value).test_err()?;
        }
        let original = serde_json::to_value(declared()).test()?;
        for pointer in ["", "/resolved", "/resolved/services/Worker.api_1"] {
            let mut value = original.clone();
            value
                .pointer_mut(pointer)
                .test()?
                .as_object_mut()
                .test()?
                .insert("unknown".into(), serde_json::json!(true));
            serde_json::from_value::<Contract>(value).test_err()?;
        }
        for replicas in [serde_json::json!(-1), serde_json::json!(1.5)] {
            let mut value = original.clone();
            value["resolved"]["services"]["Worker.api_1"]["replicas"] = replicas;
            serde_json::from_value::<Contract>(value).test_err()?;
        }
        Ok(())
    }
}
