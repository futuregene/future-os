//! Canonical JSON hashing without copying executable bodies into temporary JSON.
use serde::{ser::SerializeMap, ser::SerializeSeq, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{self, Write};

struct Canonical<'a>(&'a Value);
impl Serialize for Canonical<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(fields) => {
                let mut ordered: Vec<_> = fields.iter().collect();
                ordered.sort_by_key(|(key, _)| *key);
                let mut map = serializer.serialize_map(Some(ordered.len()))?;
                for (key, value) in ordered {
                    map.serialize_entry(key, &Canonical(value))?;
                }
                map.end()
            }
            Value::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    seq.serialize_element(&Canonical(value))?;
                }
                seq.end()
            }
            value => value.serialize(serializer),
        }
    }
}

struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn hash(value: &impl Serialize) -> String {
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value).expect("canonical action JSON");
    format!("sha256:{:x}", writer.0.finalize())
}

pub(super) fn action_digest(
    tool: &str,
    tool_id: &str,
    arguments: &Value,
    boundary: &Value,
    cwd: &str,
) -> String {
    // Alphabetical field order matches the original canonical action digest.
    #[derive(Serialize)]
    struct Binding<'a> {
        arguments: Canonical<'a>,
        boundary: Canonical<'a>,
        cwd: &'a str,
        tool: &'a str,
        tool_id: &'a str,
    }
    hash(&Binding {
        arguments: Canonical(arguments),
        boundary: Canonical(boundary),
        cwd,
        tool,
        tool_id,
    })
}

pub(super) fn denial_arguments_digest(arguments: &Value) -> String {
    struct Executable<'a>(&'a Value);
    impl Serialize for Executable<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let Some(fields) = self.0.as_object() else {
                return Canonical(self.0).serialize(serializer);
            };
            let mut ordered: Vec<_> = fields
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "justification" | "failure_summary"))
                .collect();
            ordered.sort_by_key(|(key, _)| *key);
            let mut map = serializer.serialize_map(Some(ordered.len()))?;
            for (key, value) in ordered {
                if key == "command" {
                    if let Some(command) = value.as_str() {
                        map.serialize_entry(key, command.trim())?;
                        continue;
                    }
                }
                map.serialize_entry(key, &Canonical(value))?;
            }
            map.end()
        }
    }
    hash(&Executable(arguments))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn the_canonical_writer_hashes_every_byte_it_is_given() {
        let mut writer = HashWriter(Sha256::new());
        assert_eq!(writer.write(b"canonical").unwrap(), "canonical".len());
        writer
            .flush()
            .expect("flushing the canonical writer is infallible");
        assert_eq!(
            format!("{:x}", writer.0.finalize()),
            format!("{:x}", Sha256::digest(b"canonical")),
        );
    }

    #[test]
    fn executable_arguments_that_are_not_an_object_keep_the_canonical_form() {
        // Only an object can be re-ordered; anything else must fall back to the
        // canonical encoding rather than be dropped from the denial bucket.
        let array = serde_json::json!(["printf", "second"]);
        let first = denial_arguments_digest(&array);
        assert!(first.starts_with("sha256:"));
        assert_eq!(first, denial_arguments_digest(&array));
        assert_ne!(
            first,
            denial_arguments_digest(&serde_json::json!(["printf", "third"]))
        );

        // The object form orders by key and ignores the free-text fields, so two
        // spellings of the same executable arguments share a bucket.
        assert_eq!(
            denial_arguments_digest(&serde_json::json!({"cwd": "/w", "command": "pwd"})),
            denial_arguments_digest(
                &serde_json::json!({"command": "pwd", "cwd": "/w", "justification": "why"})
            ),
        );
    }

    #[test]
    fn a_non_string_command_takes_the_generic_canonical_path() {
        // `command` is hashed trimmed when it is a string; any other JSON type
        // takes the same canonical path as every other field — and stays
        // distinguishable from the string spelling.
        assert_ne!(
            denial_arguments_digest(&serde_json::json!({"command": 7})),
            denial_arguments_digest(&serde_json::json!({"command": "7"})),
        );
        assert_eq!(
            denial_arguments_digest(&serde_json::json!({"command": "  pwd  "})),
            denial_arguments_digest(&serde_json::json!({"command": "pwd"})),
            "a string command is hashed trimmed",
        );
    }
}
