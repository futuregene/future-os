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
