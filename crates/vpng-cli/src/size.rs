//! Human-friendly byte-size parsing.
//!
//! ADR-0006: bare `--max-size` numbers and the `MB`/`KB` suffixes are decimal
//! (powers of 1000), matching the roadmap's `10,000,000`-byte ceiling. The
//! `MiB`/`KiB` suffixes and the separate `--max-size-mib` option are binary.

use vpng_core::{Error, Result};

/// Parse a size string such as `10000000`, `10MB`, or `10MiB`.
pub fn parse_size(text: &str) -> Result<u64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::invalid("empty size value"));
    }
    let lowered = trimmed.to_ascii_lowercase();

    let (digits, multiplier) = if let Some(rest) = lowered.strip_suffix("mib") {
        (rest, 1024u64 * 1024)
    } else if let Some(rest) = lowered.strip_suffix("mb") {
        (rest, 1_000_000)
    } else if let Some(rest) = lowered.strip_suffix("kib") {
        (rest, 1024)
    } else if let Some(rest) = lowered.strip_suffix("kb") {
        (rest, 1_000)
    } else if let Some(rest) = lowered.strip_suffix('b') {
        (rest, 1)
    } else {
        (lowered.as_str(), 1)
    };

    let value: u64 = digits
        .trim()
        .parse()
        .map_err(|_| Error::invalid(format!("invalid size `{text}`")))?;

    vpng_core::checked_mul(value, multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_suffixes() {
        assert_eq!(parse_size("10000000").unwrap(), 10_000_000);
        assert_eq!(parse_size("10MB").unwrap(), 10_000_000);
        assert_eq!(parse_size("10mb").unwrap(), 10_000_000);
        assert_eq!(parse_size("5KB").unwrap(), 5_000);
    }

    #[test]
    fn parses_binary_suffixes() {
        assert_eq!(parse_size("10MiB").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("2KiB").unwrap(), 2048);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_size("ten").is_err());
        assert!(parse_size("").is_err());
    }
}
