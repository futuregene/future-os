//! The CLI's one argument parser.
//!
//! Every group used to parse its own arguments, which made the same mistake
//! report differently in each one: `future mcp list --jsno` was refused while
//! `future task list --jsno` silently ignored the flag and printed the
//! plausible result an agent then trusted. AXI principle 6 asks for the same
//! guarantee an unknown *command* already has — an invented flag must fail
//! loudly, never be ignored — so the parser lives here and refuses anything it
//! was not told about.
//!
//! Parsed `future` arguments: positional values, `--flag value` pairs,
//! repeatable flags and bare switches.

use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Flags {
    pub positional: Vec<String>,
    values: BTreeMap<String, String>,
    lists: BTreeMap<String, Vec<String>>,
    switches: Vec<String>,
}

/// Parse `--key value` pairs, `--key=value`, repeatable flags into `lists`, and
/// bare switches into `switches`. An option in none of the three lists is an
/// error, which is the whole point: the caller states its surface, and anything
/// else the user (or an agent) typed is reported instead of dropped.
pub fn parse_flags(
    args: &[String],
    taking_value: &[&str],
    repeatable: &[&str],
    switches: &[&str],
) -> Result<Flags, String> {
    let mut parsed = Flags {
        positional: Vec::new(),
        values: BTreeMap::new(),
        lists: BTreeMap::new(),
        switches: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if !arg.starts_with("--") {
            parsed.positional.push(arg.to_string());
            index += 1;
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg, None),
        };
        if switches.contains(&flag) {
            if inline.is_some() {
                return Err(format!("{flag} does not take a value"));
            }
            parsed.switches.push(flag.to_string());
            index += 1;
            continue;
        }
        if !taking_value.contains(&flag) && !repeatable.contains(&flag) {
            return Err(format!("unknown option: {flag}"));
        }
        let value = match inline {
            Some(value) => value,
            None => {
                // Repeatable flags take their value verbatim: `--arg --mcp` is
                // how an MCP server is spawned, so a leading `--` is data here,
                // not the next option. Single-value flags keep the guard, where
                // a following `--flag` means the value was forgotten.
                let next = args.get(index + 1);
                let value = if repeatable.contains(&flag) {
                    next.ok_or_else(|| format!("missing value for {flag}"))?
                } else {
                    next.filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| format!("missing value for {flag}"))?
                };
                index += 1;
                value.clone()
            }
        };
        if repeatable.contains(&flag) {
            parsed
                .lists
                .entry(flag.to_string())
                .or_default()
                .push(value);
        } else {
            parsed.values.insert(flag.to_string(), value);
        }
        index += 1;
    }
    Ok(parsed)
}

impl Flags {
    pub fn has(&self, flag: &str) -> bool {
        self.switches.iter().any(|switch| switch == flag)
    }

    pub fn value(&self, flag: &str) -> Option<&str> {
        self.values.get(flag).map(String::as_str)
    }

    pub fn list(&self, flag: &str) -> &[String] {
        self.lists.get(flag).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unknown_option_is_refused_not_ignored() {
        // The regression this module exists for: a typo must not read as a
        // successful run.
        let error = parse_flags(&args(&["--jsno"]), &[], &[], &["--json"]).unwrap_err();
        assert_eq!(error, "unknown option: --jsno");
    }

    #[test]
    fn values_switches_and_repeatable_flags() {
        let parsed = parse_flags(
            &args(&["--format", "toon", "--json=x"]),
            &["--format"],
            &[],
            &["--json"],
        )
        .unwrap_err();
        assert_eq!(parsed, "--json does not take a value");

        let parsed = parse_flags(
            &args(&["p1", "--format=toon", "--all"]),
            &["--format"],
            &[],
            &["--all"],
        )
        .unwrap();
        assert_eq!(parsed.positional, vec!["p1"]);
        assert_eq!(parsed.value("--format"), Some("toon"));
        assert!(parsed.has("--all"));

        // A repeatable flag takes a value verbatim, dash and all.
        let parsed = parse_flags(&args(&["--arg", "--mcp"]), &[], &["--arg"], &[]).unwrap();
        assert_eq!(parsed.list("--arg"), ["--mcp"]);
    }

    #[test]
    fn a_forgotten_value_is_an_error() {
        let error = parse_flags(&args(&["--format"]), &["--format"], &[], &[]).unwrap_err();
        assert_eq!(error, "missing value for --format");
        let error = parse_flags(&args(&["--format", "--json"]), &["--format"], &[], &[]);
        assert!(
            error.is_err(),
            "a following flag means the value is missing"
        );
    }
}
