//! Keyboard/mouse helpers — port of `cli/src/browser/input/keyboard.ts` and
//! `cli/src/browser/input/mouse.ts`.

// ── Modifiers ───────────────────────────────────────────────────────

/// `Modifiers` — CDP Input.dispatchKeyEvent modifier bitmask.
pub const MOD_NONE: i64 = 0;
pub const MOD_ALT: i64 = 1;
pub const MOD_CONTROL: i64 = 2;
pub const MOD_META: i64 = 4;
pub const MOD_SHIFT: i64 = 8;

// ── ParsedKey ───────────────────────────────────────────────────────

/// `ParsedKey` — one CDP Input.dispatchKeyEvent payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedKey {
    pub key: String,
    pub code: String,
    pub text: String,
    pub windows_virtual_key_code: i64,
    pub native_virtual_key_code: i64,
    pub modifiers: i64,
    pub r#type: &'static str, // "keyDown" | "keyUp" | "char"
}

/// `KEY_TABLE` — the key names the current press command uses.
fn key_table(name: &str) -> Option<(&'static str, &'static str, i64, i64)> {
    // (key, code, windowsVirtualKeyCode, nativeVirtualKeyCode)
    let entry: (&str, &str, i64, i64) = match name {
        "enter" => ("Enter", "Enter", 13, 36),
        "tab" => ("Tab", "Tab", 9, 48),
        "escape" => ("Escape", "Escape", 27, 53),
        "backspace" => ("Backspace", "Backspace", 8, 51),
        "delete" => ("Delete", "Delete", 46, 117),
        "space" => (" ", "Space", 32, 49),
        "arrowup" => ("ArrowUp", "ArrowUp", 38, 126),
        "arrowdown" => ("ArrowDown", "ArrowDown", 40, 125),
        "arrowleft" => ("ArrowLeft", "ArrowLeft", 37, 123),
        "arrowright" => ("ArrowRight", "ArrowRight", 39, 124),
        "home" => ("Home", "Home", 36, 115),
        "end" => ("End", "End", 35, 119),
        "pageup" => ("PageUp", "PageUp", 33, 116),
        "pagedown" => ("PageDown", "PageDown", 34, 121),
        "shift" => ("Shift", "ShiftLeft", 16, 56),
        "control" => ("Control", "ControlLeft", 17, 59),
        "alt" => ("Alt", "AltLeft", 18, 58),
        "meta" => ("Meta", "MetaLeft", 91, 55),
        "a" => ("a", "KeyA", 65, 0),
        "c" => ("c", "KeyC", 67, 8),
        "v" => ("v", "KeyV", 86, 9),
        "x" => ("x", "KeyX", 88, 7),
        _ => return None,
    };
    Some(entry)
}

/// `parseKey(raw)` — "Enter" or "Control+A" → [keyDown, keyUp].
pub fn parse_key(raw: &str) -> Result<Vec<ParsedKey>, String> {
    let parts: Vec<&str> = raw.split('+').collect();
    if parts.len() == 1 {
        let (key, code, win, native) =
            key_table(&raw.to_lowercase()).ok_or_else(|| format!("Unknown key: \"{raw}\""))?;
        let text = if key.chars().count() == 1 {
            key.to_string()
        } else {
            String::new()
        };
        return Ok(vec![
            ParsedKey {
                key: key.to_string(),
                code: code.to_string(),
                text: text.clone(),
                windows_virtual_key_code: win,
                native_virtual_key_code: native,
                modifiers: MOD_NONE,
                r#type: "keyDown",
            },
            ParsedKey {
                key: key.to_string(),
                code: code.to_string(),
                text,
                windows_virtual_key_code: win,
                native_virtual_key_code: native,
                modifiers: MOD_NONE,
                r#type: "keyUp",
            },
        ]);
    }

    // Combo key: e.g. Control+A, Meta+C
    let modifiers = &parts[..parts.len() - 1];
    let final_key = parts[parts.len() - 1].trim();

    let mut modifier_mask = MOD_NONE;
    for mod_name in modifiers {
        let lower = mod_name.trim().to_lowercase();
        if lower == "control" || lower == "ctrl" {
            modifier_mask |= MOD_CONTROL;
        } else if lower == "shift" {
            modifier_mask |= MOD_SHIFT;
        } else if lower == "alt" || lower == "option" {
            modifier_mask |= MOD_ALT;
        } else if lower == "meta" || lower == "command" || lower == "cmd" {
            modifier_mask |= MOD_META;
        }
    }

    let (key, code, win, native) = key_table(&final_key.to_lowercase())
        .ok_or_else(|| format!("Unknown key in combo: \"{final_key}\""))?;
    let text = if key.chars().count() == 1 {
        key.to_string()
    } else {
        String::new()
    };
    Ok(vec![
        ParsedKey {
            key: key.to_string(),
            code: code.to_string(),
            text: text.clone(),
            windows_virtual_key_code: win,
            native_virtual_key_code: native,
            modifiers: modifier_mask,
            r#type: "keyDown",
        },
        ParsedKey {
            key: key.to_string(),
            code: code.to_string(),
            text,
            windows_virtual_key_code: win,
            native_virtual_key_code: native,
            modifiers: modifier_mask,
            r#type: "keyUp",
        },
    ])
}

/// `charKeyDownUp(char)`.
pub fn char_key_down_up(char: &str) -> Vec<ParsedKey> {
    let win = char.chars().next().map(|c| c as i64).unwrap_or(0);
    vec![
        ParsedKey {
            key: char.to_string(),
            code: String::new(),
            text: char.to_string(),
            windows_virtual_key_code: win,
            native_virtual_key_code: 0,
            modifiers: MOD_NONE,
            r#type: "keyDown",
        },
        ParsedKey {
            key: char.to_string(),
            code: String::new(),
            text: char.to_string(),
            windows_virtual_key_code: win,
            native_virtual_key_code: 0,
            modifiers: MOD_NONE,
            r#type: "keyUp",
        },
    ]
}

// ── Mouse ───────────────────────────────────────────────────────────

/// `ElementBox`.
#[derive(Debug, Clone, Copy)]
pub struct ElementBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `centerOf(box)` — Math.round of the box center.
pub fn center_of(box_: ElementBox) -> (i64, i64) {
    (
        (box_.x + box_.width / 2.0).round() as i64,
        (box_.y + box_.height / 2.0).round() as i64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_key_returns_keydown_and_keyup() {
        let keys = parse_key("Enter").unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].r#type, "keyDown");
        assert_eq!(keys[0].key, "Enter");
        assert_eq!(keys[0].code, "Enter");
        assert_eq!(keys[0].modifiers, MOD_NONE);
        assert_eq!(keys[1].r#type, "keyUp");
    }

    #[test]
    fn tab_produces_correct_key_up_pair() {
        let keys = parse_key("Tab").unwrap();
        assert_eq!(keys[0].key, "Tab");
        assert_eq!(keys[0].windows_virtual_key_code, 9);
    }

    #[test]
    fn escape_backspace_space() {
        assert_eq!(parse_key("Escape").unwrap()[0].key, "Escape");
        assert_eq!(parse_key("Backspace").unwrap()[0].key, "Backspace");
        assert_eq!(parse_key("Space").unwrap()[0].key, " ");
    }

    #[test]
    fn arrow_keys_work() {
        for key in ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"] {
            let keys = parse_key(key).unwrap();
            assert_eq!(keys[0].key, key);
            assert_eq!(keys[0].code, key);
        }
    }

    #[test]
    fn control_a_modifier_combination() {
        let keys = parse_key("Control+A").unwrap();
        assert_eq!(keys[0].modifiers, MOD_CONTROL);
        assert_eq!(keys[0].key, "a");
        assert_eq!(keys[0].code, "KeyA");
    }

    #[test]
    fn meta_c_modifier_combination() {
        let keys = parse_key("Meta+C").unwrap();
        assert_eq!(keys[0].modifiers, MOD_META);
    }

    #[test]
    fn control_shift_tab_combo() {
        let keys = parse_key("Control+Shift+Tab").unwrap();
        assert_eq!(keys[0].modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(keys[0].key, "Tab");
    }

    #[test]
    fn unknown_key_throws() {
        assert!(parse_key("F23").is_err());
    }

    #[test]
    fn home_end_pageup_pagedown_work() {
        for key in ["Home", "End", "PageUp", "PageDown"] {
            assert!(parse_key(key).is_ok());
        }
    }

    #[test]
    fn center_of_simple_box() {
        assert_eq!(
            center_of(ElementBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0
            }),
            (50, 25)
        );
    }

    #[test]
    fn center_with_offset_origin() {
        assert_eq!(
            center_of(ElementBox {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 60.0
            }),
            (30, 50)
        );
    }

    #[test]
    fn center_rounds_to_integer() {
        // 7/2 = 3.5 → 4 (Math.round), 3/2 = 1.5 → 2
        assert_eq!(
            center_of(ElementBox {
                x: 0.0,
                y: 0.0,
                width: 7.0,
                height: 3.0
            }),
            (4, 2)
        );
    }

    #[test]
    fn center_zero_size_box() {
        assert_eq!(
            center_of(ElementBox {
                x: 5.0,
                y: 5.0,
                width: 0.0,
                height: 0.0
            }),
            (5, 5)
        );
    }

    #[test]
    fn center_negative_coordinates() {
        assert_eq!(
            center_of(ElementBox {
                x: -10.0,
                y: -20.0,
                width: 40.0,
                height: 60.0
            }),
            (10, 10)
        );
    }

    #[test]
    fn remaining_key_table_entries() {
        assert_eq!(parse_key("Delete").unwrap()[0].windows_virtual_key_code, 46);
        assert_eq!(parse_key("Shift").unwrap()[0].key, "Shift");
        assert_eq!(parse_key("Control").unwrap()[0].key, "Control");
        assert_eq!(parse_key("Alt").unwrap()[0].key, "Alt");
        assert_eq!(parse_key("Meta").unwrap()[0].key, "Meta");
        assert_eq!(parse_key("v").unwrap()[0].code, "KeyV");
        assert_eq!(parse_key("x").unwrap()[0].code, "KeyX");
    }

    #[test]
    fn single_char_key_carries_text() {
        let keys = parse_key("a").unwrap();
        assert_eq!(keys[0].text, "a");
        assert_eq!(keys[1].text, "a");
        // Multi-char key names carry no text.
        assert_eq!(parse_key("Enter").unwrap()[0].text, "");
    }

    #[test]
    fn modifier_aliases() {
        assert_eq!(parse_key("Ctrl+A").unwrap()[0].modifiers, MOD_CONTROL);
        assert_eq!(parse_key("Option+A").unwrap()[0].modifiers, MOD_ALT);
        assert_eq!(parse_key("Command+A").unwrap()[0].modifiers, MOD_META);
        assert_eq!(parse_key("Cmd+A").unwrap()[0].modifiers, MOD_META);
        // Unknown modifier names are ignored (JS permissiveness).
        assert_eq!(parse_key("Hyper+A").unwrap()[0].modifiers, MOD_NONE);
    }

    #[test]
    fn combo_with_unknown_final_key_errors() {
        let err = parse_key("Control+F23").unwrap_err();
        assert_eq!(err, "Unknown key in combo: \"F23\"");
        let err = parse_key("F23").unwrap_err();
        assert_eq!(err, "Unknown key: \"F23\"");
    }

    #[test]
    fn char_key_down_up_emits_char_codes() {
        let keys = char_key_down_up("Q");
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].r#type, "keyDown");
        assert_eq!(keys[1].r#type, "keyUp");
        assert_eq!(keys[0].key, "Q");
        assert_eq!(keys[0].text, "Q");
        assert_eq!(keys[0].windows_virtual_key_code, 'Q' as i64);
        assert_eq!(keys[0].native_virtual_key_code, 0);
        // Empty string → code 0 (defensive; JS would produce NaN semantics).
        let empty = char_key_down_up("");
        assert_eq!(empty[0].windows_virtual_key_code, 0);
    }
}
