//! Versioned interpretation of interrupted tool-input strings. This is shared
//! by the model loop and canonical reference readers; stored raw bytes stay
//! immutable, and reads never create a second repaired argument body.
pub fn repair_string_v1(args: &mut serde_json::Value) {
    let serde_json::Value::String(raw) = args else {
        return;
    };
    // Empty string is never valid tool-call arguments — treat as empty object
    // so the tool handler gets a proper "missing field" error instead of
    // "invalid type: string \"\", expected struct ShellParams".
    if raw.is_empty() {
        *raw = String::from("{}");
        return;
    }
    if valid_json(raw) {
        return;
    }
    let Some(repaired) = repair_partial_json_object(raw) else {
        return;
    };
    if valid_json(&repaired) {
        *raw = repaired;
    }
}

/// Match Value's syntax/number/recursion validation without retaining every
/// array/map element solely to decide whether a repaired string is valid JSON.
fn valid_json(raw: &str) -> bool {
    use serde::Deserialize;
    struct Validate;
    impl<'de> serde::Deserialize<'de> for Validate {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Validate;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a JSON value")
                }
                fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Validate, E> {
                    Ok(Validate)
                }
                fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Validate, E> {
                    Ok(Validate)
                }
                fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Validate, E> {
                    Ok(Validate)
                }
                fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Validate, E> {
                    if value.is_finite() {
                        Ok(Validate)
                    } else {
                        Err(E::custom("JSON number out of range"))
                    }
                }
                fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Validate, E> {
                    Ok(Validate)
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Validate, E> {
                    Ok(Validate)
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Validate, A::Error> {
                    while seq.next_element::<Validate>()?.is_some() {}
                    Ok(Validate)
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Validate, A::Error> {
                    while map.next_key::<Validate>()?.is_some() {
                        map.next_value::<Validate>()?;
                    }
                    Ok(Validate)
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    let mut parser = serde_json::Deserializer::from_str(raw);
    Validate::deserialize(&mut parser)
        .and_then(|_| parser.end())
        .is_ok()
}

pub fn repair_partial_json_object(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') {
        return None;
    }

    let mut repaired = trimmed.to_string();
    if has_unclosed_string(&repaired) {
        repaired.push('"');
    }

    let open_braces = repaired.chars().filter(|c| *c == '{').count();
    let close_braces = repaired.chars().filter(|c| *c == '}').count();
    if open_braces > close_braces {
        for _ in 0..(open_braces - close_braces) {
            repaired.push('}');
        }
    }

    Some(repaired)
}

pub fn has_unclosed_string(value: &str) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            _ => {}
        }
    }
    in_string
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_light_validation_keeps_value_number_string_and_recursion_semantics() {
        for raw in [
            "null",
            "true",
            "false",
            "0",
            "-0",
            "1e300",
            "1e400",
            "{\"a\":1e400}",
            "[1,2,3]",
            "[1,]",
            "{} x",
            "{\"a\":true,\"a\":null}",
            r#""\uD834\uDD1E""#,
            r#""\uD834""#,
            "[{}]",
        ] {
            assert_eq!(
                valid_json(raw),
                serde_json::from_str::<serde_json::Value>(raw).is_ok(),
                "{raw}"
            );
        }
        for depth in [100, 127, 128, 129] {
            let raw = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(
                valid_json(&raw),
                serde_json::from_str::<serde_json::Value>(&raw).is_ok()
            );
        }
        let raw = format!("[{}]", vec!["{\"escaped\":\"中\\n🙂\"}"; 20000].join(","));
        assert!(valid_json(&raw));
    }
    #[test]
    fn invalid_repaired_numbers_are_not_accepted_and_valid_repair_stays_a_string() {
        let mut invalid = serde_json::json!("{\"n\":1e400");
        repair_string_v1(&mut invalid);
        assert_eq!(invalid, "{\"n\":1e400");
        let mut partial = serde_json::json!("{\"path\":\"中");
        repair_string_v1(&mut partial);
        assert_eq!(partial, serde_json::json!("{\"path\":\"中\"}"));
        let mut empty = serde_json::json!("");
        repair_string_v1(&mut empty);
        assert_eq!(empty, "{}");
    }
}
