use std::collections::{BTreeMap, BTreeSet};

use crate::template::{TemplateContext, render_template, template_keys};

use super::{ConfigError, StacksteadConfig, helpers::invalid};

impl StacksteadConfig {
    pub fn resolve_urls(
        &self,
        context: &mut TemplateContext,
    ) -> Result<BTreeMap<String, String>, ConfigError> {
        let mut pending = self
            .resources
            .ports
            .expose
            .iter()
            .filter_map(|(service, exposure)| {
                exposure
                    .url
                    .as_deref()
                    .map(|template| (service.clone(), template))
            })
            .map(|(service, template)| {
                let keys: BTreeSet<String> = template_keys(template).map_err(|error| {
                    ConfigError::Validation(format!("URL for `{service}`: {error}"))
                })?;
                Ok((service, (template, keys)))
            })
            .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
        let mut urls = BTreeMap::new();
        while !pending.is_empty() {
            let ready = pending
                .iter()
                .find(|(_, (_, keys))| keys.iter().all(|key| context.contains_key(key)))
                .map(|(service, _)| service.clone());
            let Some(service) = ready else {
                return invalid(format!(
                    "cyclic or unresolved URL template references: {}",
                    pending
                        .keys()
                        .map(|name| format!("urls.{name}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            let (template, _) = pending
                .remove(&service)
                .ok_or_else(|| ConfigError::Validation("resolved URL disappeared".into()))?;
            let url = render_template(template, context).map_err(|error| {
                ConfigError::Validation(format!("URL for `{service}`: {error}"))
            })?;
            if url.chars().any(char::is_whitespace) || !crate::open::is_loopback_url(&url) {
                return invalid(format!(
                    "URL for `{service}` must render a loopback http:// or https:// URL without credentials or whitespace"
                ));
            }
            context.insert(format!("urls.{service}"), url.clone());
            urls.insert(service, url);
        }
        Ok(urls)
    }
}
