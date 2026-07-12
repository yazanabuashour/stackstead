use std::error::Error;
use std::fmt;
use std::path::Path;

const RANDOM_ID_BYTES: usize = 16;
pub const SHORT_ID_LEN: usize = RANDOM_ID_BYTES * 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugError {
    Empty,
    DotName,
    AbsolutePath,
    PathSeparator,
    UnsafeCharacter(char),
    EmptyAfterSanitization,
    InvalidShortId,
}

impl fmt::Display for SlugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "stackstead name cannot be empty"),
            Self::DotName => write!(f, "stackstead name cannot be `.` or `..`"),
            Self::AbsolutePath => write!(f, "stackstead name cannot be an absolute path"),
            Self::PathSeparator => write!(f, "stackstead name cannot contain path separators"),
            Self::UnsafeCharacter(character) => {
                write!(f, "stackstead name contains unsafe character {character:?}")
            }
            Self::EmptyAfterSanitization => {
                write!(f, "stackstead name has no usable slug characters")
            }
            Self::InvalidShortId => write!(
                f,
                "stackstead short id must contain exactly {SHORT_ID_LEN} hexadecimal characters"
            ),
        }
    }
}

impl Error for SlugError {}

pub fn sanitize_slug(name: &str) -> Result<String, SlugError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SlugError::Empty);
    }
    if matches!(name, "." | "..") {
        return Err(SlugError::DotName);
    }
    if Path::new(name).is_absolute() {
        return Err(SlugError::AbsolutePath);
    }
    if name.contains(['/', '\\']) {
        return Err(SlugError::PathSeparator);
    }

    let mut slug = String::with_capacity(name.len());
    let mut pending_separator = false;

    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            pending_separator = false;
        } else if matches!(character, '-' | '_' | '.') || character == ' ' {
            pending_separator = true;
        } else {
            return Err(SlugError::UnsafeCharacter(character));
        }
    }

    if slug.is_empty() {
        return Err(SlugError::EmptyAfterSanitization);
    }

    Ok(slug)
}

pub fn new_short_id() -> anyhow::Result<String> {
    new_random_hex()
}

pub(crate) fn new_random_hex() -> anyhow::Result<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0_u8; RANDOM_ID_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("cannot generate a secure runtime identity: {error}"))?;
    let mut value = String::with_capacity(RANDOM_ID_BYTES * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(value)
}

pub fn make_stackstead_id(name: &str, short_id: &str) -> Result<String, SlugError> {
    let slug = sanitize_slug(name)?;
    let short_id = normalize_short_id(short_id)?;
    Ok(format!("{slug}-{short_id}"))
}

pub fn normalize_short_id(short_id: &str) -> Result<String, SlugError> {
    if short_id.len() != SHORT_ID_LEN || !short_id.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err(SlugError::InvalidShortId);
    }
    Ok(short_id.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_human_names() {
        assert_eq!(
            sanitize_slug(" Fix Checkout_timeout... ").unwrap(),
            "fix-checkout-timeout"
        );
        assert_eq!(sanitize_slug("feature-a").unwrap(), "feature-a");
    }

    #[test]
    fn rejects_dangerous_names() {
        for name in ["", " ", ".", "..", "/tmp/cell", "feature/a", "feature\\a"] {
            assert!(sanitize_slug(name).is_err(), "accepted {name:?}");
        }
        for name in [
            "feature;rm",
            "$(command)",
            "feature|other",
            "feature\nother",
        ] {
            assert!(
                matches!(sanitize_slug(name), Err(SlugError::UnsafeCharacter(_))),
                "accepted {name:?}"
            );
        }
    }

    #[test]
    fn rejects_non_ascii_and_unrecognized_punctuation() {
        assert_eq!(sanitize_slug("café"), Err(SlugError::UnsafeCharacter('é')));
        assert_eq!(
            sanitize_slug("feature:one"),
            Err(SlugError::UnsafeCharacter(':'))
        );
    }

    #[test]
    fn creates_normalized_stackstead_id() {
        let short_id = "A17C0123456789ABCDEF0123456789AB";
        assert_eq!(
            make_stackstead_id("Feature A", short_id).unwrap(),
            "feature-a-a17c0123456789abcdef0123456789ab"
        );
        assert_eq!(
            make_stackstead_id("feature-a", "xyz1"),
            Err(SlugError::InvalidShortId)
        );
    }

    #[test]
    fn generated_short_ids_have_the_contract_shape() {
        let id = new_short_id().unwrap();
        assert_eq!(id.len(), SHORT_ID_LEN);
        assert!(
            id.bytes()
                .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
        );
    }

    #[test]
    fn rejects_correctly_sized_non_hex_short_id() {
        assert_eq!(
            normalize_short_id("0123456789abcdef0123456789abcdeg"),
            Err(SlugError::InvalidShortId)
        );
    }
}
