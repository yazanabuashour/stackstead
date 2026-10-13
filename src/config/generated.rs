use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use crate::{
    envfile,
    template::{TemplateContext, render_template},
};

use super::{CONFIG_FILE, ConfigError, StacksteadConfig, helpers::invalid};

impl StacksteadConfig {
    pub(super) fn validate_generated_paths(&self) -> Result<(), ConfigError> {
        let normalize = |path: &Path| -> PathBuf {
            path.components()
                .filter(|part| !matches!(part, Component::CurDir))
                .collect()
        };
        let outputs = [
            ("env.file", normalize(&self.env.file)),
            ("agent.context_file", normalize(&self.agent.context_file)),
            (
                "reciprocal pointer",
                PathBuf::from(".stackstead/stackstead.json"),
            ),
            (
                "Compose ownership override",
                PathBuf::from(crate::compose::OWNERSHIP_OVERRIDE),
            ),
        ];
        let inputs = [PathBuf::from(CONFIG_FILE), PathBuf::from(".git")]
            .into_iter()
            .chain(self.runtime.files.iter().map(|path| normalize(path)));
        let overlaps =
            |left: &Path, right: &Path| left.starts_with(right) || right.starts_with(left);
        for input in inputs {
            for (name, output) in &outputs {
                if overlaps(output, &input) {
                    return invalid(format!(
                        "{name} overlaps protected source input {}",
                        input.display()
                    ));
                }
            }
        }
        for (index, (name, output)) in outputs.iter().enumerate() {
            for (other_name, other_output) in outputs.iter().skip(index.saturating_add(1)) {
                if overlaps(output, other_output) {
                    return invalid(format!("{name} overlaps {other_name}"));
                }
            }
        }
        Ok(())
    }

    pub fn generated_environment(
        &self,
        context: &TemplateContext,
    ) -> Result<BTreeMap<String, String>, ConfigError> {
        self.env
            .generate
            .iter()
            .map(|(key, template)| {
                let value = render_template(template, context).map_err(|error| {
                    ConfigError::Validation(format!("env.generate.{key}: {error}"))
                })?;
                envfile::validate_value(&value).map_err(|error| {
                    ConfigError::Validation(format!("env.generate.{key}: {error}"))
                })?;
                Ok((key.clone(), value))
            })
            .collect()
    }
}
