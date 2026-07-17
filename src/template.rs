use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub type TemplateContext = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    EmptyKey,
    InvalidKey(String),
    UnclosedExpression,
    UnexpectedClosingDelimiter,
    UnknownKey(String),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyKey => write!(f, "template expression has an empty key"),
            Self::InvalidKey(key) => write!(f, "invalid template key `{key}`"),
            Self::UnclosedExpression => write!(f, "template expression is missing `}}}}`"),
            Self::UnexpectedClosingDelimiter => {
                write!(f, "template contains `}}}}` without a matching `{{{{`")
            }
            Self::UnknownKey(key) => write!(f, "unknown template key `{key}`"),
        }
    }
}

impl Error for TemplateError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplatePart<'a> {
    Text(&'a str),
    Key(&'a str),
}

pub fn render_template(template: &str, context: &TemplateContext) -> Result<String, TemplateError> {
    let mut rendered = String::with_capacity(template.len());
    for part in parse_template(template)? {
        match part {
            TemplatePart::Text(text) => rendered.push_str(text),
            TemplatePart::Key(key) => rendered.push_str(
                context
                    .get(key)
                    .ok_or_else(|| TemplateError::UnknownKey(key.to_owned()))?,
            ),
        }
    }
    Ok(rendered)
}

pub fn template_keys(template: &str) -> Result<BTreeSet<String>, TemplateError> {
    Ok(parse_template(template)?
        .into_iter()
        .filter_map(|part| match part {
            TemplatePart::Key(key) => Some(key.to_owned()),
            TemplatePart::Text(_) => None,
        })
        .collect())
}

pub fn validate_template_keys<'a>(
    template: &str,
    allowed_keys: impl IntoIterator<Item = &'a str>,
) -> Result<(), TemplateError> {
    let allowed: BTreeSet<&str> = allowed_keys.into_iter().collect();
    for key in template_keys(template)? {
        if !allowed.contains(key.as_str()) {
            return Err(TemplateError::UnknownKey(key));
        }
    }
    Ok(())
}

fn parse_template(template: &str) -> Result<Vec<TemplatePart<'_>>, TemplateError> {
    let mut parts = Vec::new();
    let mut cursor = 0;

    while cursor < template.len() {
        let remaining = &template[cursor..];
        let opening = remaining.find("{{");
        let closing = remaining.find("}}");

        if closing.is_some_and(|position| opening.is_none_or(|open| position < open)) {
            return Err(TemplateError::UnexpectedClosingDelimiter);
        }

        let Some(opening) = opening else {
            parts.push(TemplatePart::Text(remaining));
            break;
        };

        if opening > 0 {
            parts.push(TemplatePart::Text(&remaining[..opening]));
        }

        let expression_start = cursor + opening + 2;
        let after_opening = &template[expression_start..];
        let closing = after_opening
            .find("}}")
            .ok_or(TemplateError::UnclosedExpression)?;
        let key = after_opening[..closing].trim();

        if key.is_empty() {
            return Err(TemplateError::EmptyKey);
        }
        if !valid_key(key) {
            return Err(TemplateError::InvalidKey(key.to_owned()));
        }

        parts.push(TemplatePart::Key(key));
        cursor = expression_start + closing + 2;
    }

    Ok(parts)
}

fn valid_key(key: &str) -> bool {
    key.split('.').all(|segment| {
        let mut chars = segment.chars();
        chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && chars.all(|character| {
                character.is_ascii_alphanumeric() || character == '_' || character == '-'
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestResultExt as _;

    fn context() -> TemplateContext {
        BTreeMap::from([
            ("ports.web".to_owned(), "39100".to_owned()),
            ("project.name".to_owned(), "loan-platform".to_owned()),
        ])
    }

    #[test]
    fn renders_normal_and_repeated_keys_with_whitespace() -> anyhow::Result<()> {
        assert_eq!(
            render_template(
                "{{ project.name }}:{{ports.web}}/{{ project.name }}",
                &context()
            )
            .test()?,
            "loan-platform:39100/loan-platform"
        );
        Ok(())
    }

    #[test]
    fn leaves_plain_text_unchanged() -> anyhow::Result<()> {
        assert_eq!(
            render_template("plain text", &context()).test()?,
            "plain text"
        );
        assert_eq!(render_template("", &context()).test()?, "");
        Ok(())
    }

    #[test]
    fn renders_url_and_env_values() -> anyhow::Result<()> {
        assert_eq!(
            render_template("http://127.0.0.1:{{ ports.web }}", &context()).test()?,
            "http://127.0.0.1:39100"
        );
        assert_eq!(
            render_template("WEB_PORT={{ ports.web }}", &context()).test()?,
            "WEB_PORT=39100"
        );
        Ok(())
    }

    #[test]
    fn rejects_unknown_keys() -> anyhow::Result<()> {
        assert_eq!(
            render_template("{{ ports.api }}", &context()),
            Err(TemplateError::UnknownKey("ports.api".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn rejects_malformed_expressions() -> anyhow::Result<()> {
        assert_eq!(
            render_template("{{ ports.web", &context()),
            Err(TemplateError::UnclosedExpression)
        );
        assert_eq!(
            render_template("ports.web }}", &context()),
            Err(TemplateError::UnexpectedClosingDelimiter)
        );
        assert_eq!(
            render_template("{{ }}", &context()),
            Err(TemplateError::EmptyKey)
        );
        assert_eq!(
            render_template("{{ ports web }}", &context()),
            Err(TemplateError::InvalidKey("ports web".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn validates_keys_without_rendering() -> anyhow::Result<()> {
        assert!(
            validate_template_keys(
                "{{ project.name }}-{{ ports.web }}",
                ["project.name", "ports.web"]
            )
            .is_ok()
        );
        assert_eq!(
            validate_template_keys("{{ paths.unknown }}", ["project.name"]),
            Err(TemplateError::UnknownKey("paths.unknown".to_owned()))
        );
        Ok(())
    }
}
