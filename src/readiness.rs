mod evaluation;
mod model;

pub use evaluation::{ReadinessReport, ReadinessStatus, evaluate};
pub use model::{Contract, Requirements, Role, ServiceRequirement};

pub fn is_sha256(value: &str) -> bool {
    // SHA-256 encodes its 256 bits as exactly 64 lowercase hexadecimal characters.
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
