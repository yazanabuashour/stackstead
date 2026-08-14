use std::collections::BTreeMap;

pub fn redact(value: &str) -> String {
    redact_with_env(value, &BTreeMap::new())
}

pub fn redact_with_env(value: &str, env: &BTreeMap<String, String>) -> String {
    let mut ranges = sensitive_assignment_ranges(value);
    ranges.extend(sensitive_header_ranges(value));
    ranges.extend(credential_url_ranges(value));
    for secret in known_secret_values(env) {
        ranges.extend(
            value
                .match_indices(secret)
                .map(|(start, matched)| (start, start.saturating_add(matched.len()))),
        );
    }
    replace_ranges(value, ranges)
}

#[expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "the byte scanner checks every index and only slices at ASCII token boundaries"
)]
fn sensitive_assignment_ranges(value: &str) -> Vec<(usize, usize)> {
    let bytes = value.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !is_name_start(bytes[index]) || index > 0 && is_name_character(bytes[index - 1]) {
            index += 1;
            continue;
        }
        let name_start = index;
        index += 1;
        while index < bytes.len() && is_name_character(bytes[index]) {
            index += 1;
        }
        let name_end = index;
        if index < bytes.len() && bytes[index] == b'[' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index >= bytes.len() || bytes[index] != b']' {
                continue;
            }
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b'+' {
            index += 1;
        }
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
        if !crate::envfile::is_secret_name(&value[name_start..name_end]) {
            continue;
        }

        let start = index;
        let enclosing_quote = name_start
            .checked_sub(1)
            .and_then(|before| matches!(bytes[before], b'\'' | b'"').then_some(bytes[before]));
        let end = if index < bytes.len() && matches!(bytes[index], b'\'' | b'"') {
            quoted_end(bytes, index + 1, bytes[index], true)
        } else if let Some(quote) = enclosing_quote {
            quoted_end(bytes, index, quote, false)
        } else {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            index
        };
        ranges.push((start, end));
        index = end.max(index);
    }
    ranges
}

#[expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "the byte scanner checks every index and only slices at ASCII header boundaries"
)]
fn sensitive_header_ranges(value: &str) -> Vec<(usize, usize)> {
    const HEADERS: [&str; 5] = [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
    ];
    let bytes = value.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic()
            || index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'-')
        {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'-' | b'_'))
        {
            index += 1;
        }
        let mut separator = index;
        while separator < bytes.len() && matches!(bytes[separator], b' ' | b'\t') {
            separator += 1;
        }
        if separator >= bytes.len()
            || bytes[separator] != b':'
            || !HEADERS
                .iter()
                .any(|header| value[start..index].eq_ignore_ascii_case(header))
        {
            continue;
        }
        index = separator + 1;
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
        let value_start = index;
        let enclosing_quote = start
            .checked_sub(1)
            .and_then(|before| matches!(bytes[before], b'\'' | b'"').then_some(bytes[before]));
        let value_end = if let Some(quote) = enclosing_quote {
            quoted_end(bytes, index, quote, false)
        } else {
            while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
                index += 1;
            }
            index
        };
        ranges.push((value_start, value_end));
        index = value_end.max(index);
    }
    ranges
}

#[expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "the byte scanner checks every index and only slices at ASCII URL delimiters"
)]
fn credential_url_ranges(value: &str) -> Vec<(usize, usize)> {
    let bytes = value.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(relative) = value[index..].find("://") {
        let separator = index + relative;
        let mut scheme_start = separator;
        while scheme_start > 0 && is_scheme_character(bytes[scheme_start - 1]) {
            scheme_start -= 1;
        }
        let valid_scheme = scheme_start < separator
            && bytes[scheme_start].is_ascii_alphabetic()
            && (scheme_start == 0 || !is_scheme_character(bytes[scheme_start - 1]));
        let authority_start = separator + 3;
        let mut authority_end = authority_start;
        while authority_end < bytes.len()
            && !bytes[authority_end].is_ascii_whitespace()
            && !matches!(
                bytes[authority_end],
                b'/' | b'?' | b'#' | b'\'' | b'"' | b'<' | b'>' | b')' | b']' | b'}'
            )
        {
            authority_end += 1;
        }
        if valid_scheme && let Some(at) = value[authority_start..authority_end].rfind('@') {
            let userinfo_end = authority_start + at;
            if userinfo_end > authority_start {
                ranges.push((authority_start, userinfo_end));
            }
        }
        index = authority_end.max(separator + 3);
    }
    ranges
}

fn known_secret_values(env: &BTreeMap<String, String>) -> Vec<&str> {
    let mut secrets = env
        .iter()
        .filter(|(name, value)| !value.is_empty() && crate::envfile::should_redact(name, value))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    secrets
        .sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    secrets.dedup();
    secrets
}

#[expect(
    clippy::string_slice,
    reason = "all ranges originate from match_indices or the checked ASCII scanners above"
)]
fn replace_ranges(value: &str, mut ranges: Vec<(usize, usize)>) -> String {
    ranges.sort_unstable_by_key(|&(start, end)| (start, std::cmp::Reverse(end)));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some((_, previous_end)) = merged.last_mut()
            && start <= *previous_end
        {
            *previous_end = (*previous_end).max(end);
        } else {
            merged.push((start, end));
        }
    }
    let mut redacted = String::with_capacity(value.len());
    let mut previous_end = 0;
    for (start, end) in merged {
        redacted.push_str(&value[previous_end..start]);
        redacted.push_str("[REDACTED]");
        previous_end = end;
    }
    redacted.push_str(&value[previous_end..]);
    redacted
}

#[expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "the loop bounds every byte access and advances by one within the slice length"
)]
fn quoted_end(bytes: &[u8], mut index: usize, quote: u8, include_quote: bool) -> usize {
    let mut escaped = false;
    while index < bytes.len() {
        if bytes[index] == quote && !escaped {
            return index + usize::from(include_quote);
        }
        escaped = bytes[index] == b'\\' && !escaped;
        if bytes[index] != b'\\' {
            escaped = false;
        }
        index += 1;
    }
    bytes.len()
}

const fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

const fn is_name_character(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

const fn is_scheme_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}
