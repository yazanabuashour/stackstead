use std::{error::Error, fmt, path::PathBuf};

pub const CONFIG_FILE: &str = "stackstead.yaml";
pub const CONFIG_VERSION: &str = "1";

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse(serde_yaml::Error),
    Validation(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "failed to read config {}: {source}", path.display())
            }
            Self::Parse(source) => write!(f, "invalid Stackstead config: {source}"),
            Self::Validation(message) => write!(f, "invalid Stackstead config: {message}"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse(source) => Some(source),
            Self::Validation(_) => None,
        }
    }
}

mod features;
mod helpers;
mod model;
mod validation;

pub use features::*;
pub use helpers::reserved_process_env;
pub use model::*;

#[cfg(test)]
mod tests;
