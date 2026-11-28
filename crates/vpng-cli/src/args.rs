//! Minimal, strict command-line argument parser.
//!
//! Long options are `--key value` or `--key=value`. Only options on the known
//! lists are accepted, so a typo fails loudly instead of being ignored.

use std::collections::HashMap;

use vpng_core::{Error, Result};

/// Long options that consume the following token as their value.
const VALUE_OPTIONS: &[&str] = &[
    "x",
    "y",
    "width",
    "height",
    "tile-x",
    "tile-y",
    "output",
    "tile-size",
    "tile-width",
    "tile-height",
    "codec",
    "codec-set",
    "max-size",
    "max-size-mib",
    "format",
    "input-format",
    "cache-bytes",
    "jobs",
    "workers",
    "pyramid-levels",
    "adaptive-threshold",
    "level",
    "layer",
    "manifest",
    "port",
    "max-region",
];

/// Long options that are simple flags.
const SWITCH_OPTIONS: &[&str] = &[
    "json",
    "help",
    "verbose",
    "quiet",
    "force",
    "version",
    "pyramid",
    "adaptive",
    "gpu",
    "no-browser",
];

#[derive(Debug, Default, Clone)]
pub struct Args {
    pub command: String,
    pub positional: Vec<String>,
    pub options: HashMap<String, String>,
    pub switches: Vec<String>,
}

impl Args {
    pub fn parse(argv: &[String]) -> Result<Args> {
        let mut tokens = argv.iter();
        let command = tokens
            .next()
            .cloned()
            .ok_or_else(|| Error::invalid("missing command; run `vpng help`"))?;

        let rest: Vec<&String> = tokens.collect();
        let mut positional = Vec::new();
        let mut options = HashMap::new();
        let mut switches = Vec::new();

        let mut index = 0usize;
        while index < rest.len() {
            let token = rest[index].as_str();

            if let Some(body) = token.strip_prefix("--") {
                if let Some((key, value)) = body.split_once('=') {
                    let key = key.to_ascii_lowercase();
                    if !VALUE_OPTIONS.contains(&key.as_str()) {
                        return Err(Error::invalid(format!("unknown option `--{key}`")));
                    }
                    options.insert(key, value.to_string());
                } else {
                    let key = body.to_ascii_lowercase();
                    if VALUE_OPTIONS.contains(&key.as_str()) {
                        let next = rest.get(index + 1).ok_or_else(|| {
                            Error::invalid(format!("option --{key} requires a value"))
                        })?;
                        options.insert(key, (*next).clone());
                        index += 1;
                    } else if SWITCH_OPTIONS.contains(&key.as_str()) {
                        switches.push(key);
                    } else {
                        return Err(Error::invalid(format!("unknown option `--{key}`")));
                    }
                }
            } else if token == "-o" {
                let next = rest
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid("option -o requires a value"))?;
                options.insert("output".to_string(), (*next).clone());
                index += 1;
            } else if token == "-h" {
                switches.push("help".to_string());
            } else if token.starts_with('-') && token.len() > 1 {
                return Err(Error::invalid(format!("unknown option `{token}`")));
            } else {
                positional.push(token.to_string());
            }

            index += 1;
        }

        Ok(Args {
            command,
            positional,
            options,
            switches,
        })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.options.get(key).map(|value| value.as_str())
    }

    pub fn require(&self, key: &str) -> Result<&str> {
        self.get(key)
            .ok_or_else(|| Error::invalid(format!("missing required option --{key}")))
    }

    pub fn has(&self, key: &str) -> bool {
        self.switches.iter().any(|switch| switch == key)
    }

    pub fn get_u32(&self, key: &str) -> Result<Option<u32>> {
        match self.get(key) {
            None => Ok(None),
            Some(value) => value
                .parse::<u32>()
                .map(Some)
                .map_err(|_| Error::invalid(format!("--{key} expects an integer, got `{value}`"))),
        }
    }

    pub fn require_u32(&self, key: &str) -> Result<u32> {
        self.get_u32(key)?
            .ok_or_else(|| Error::invalid(format!("missing required option --{key}")))
    }

    pub fn positional(&self, index: usize) -> Option<&str> {
        self.positional.get(index).map(|value| value.as_str())
    }

    pub fn require_positional(&self, index: usize, what: &str) -> Result<&str> {
        self.positional(index)
            .ok_or_else(|| Error::invalid(format!("missing {what}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_values_and_switches() {
        let args = Args::parse(&argv(&[
            "encode",
            "in.png",
            "out.vpng",
            "--tile-size",
            "128",
            "--json",
        ]))
        .unwrap();
        assert_eq!(args.command, "encode");
        assert_eq!(args.positional, vec!["in.png", "out.vpng"]);
        assert_eq!(args.get("tile-size"), Some("128"));
        assert!(args.has("json"));
    }

    #[test]
    fn parses_equals_form() {
        let args = Args::parse(&argv(&["encode", "a", "b", "--max-size=10MB"])).unwrap();
        assert_eq!(args.get("max-size"), Some("10MB"));
    }

    #[test]
    fn rejects_unknown_option() {
        assert!(Args::parse(&argv(&["info", "f.vpng", "--nope"])).is_err());
    }

    #[test]
    fn missing_value_is_an_error() {
        assert!(Args::parse(&argv(&["encode", "a", "b", "--tile-size"])).is_err());
    }

    #[test]
    fn short_output_option() {
        let args = Args::parse(&argv(&["extract-tile", "f.vpng", "-o", "t.png"])).unwrap();
        assert_eq!(args.get("output"), Some("t.png"));
    }
}
