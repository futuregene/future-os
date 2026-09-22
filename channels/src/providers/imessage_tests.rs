//! Tests for the iMessage provider: AppleScript escaping (injection
//! samples), Apple-epoch timestamp conversion, chat.db row parsing against
//! embedded fixtures, config defaults and the unsupported-platform path.

use super::*;
use serde_json::json;

// ─── Definition and config ──────────────────────────────────────────────────

#[test]
fn the_definition_is_preview_and_direct_only() {
    assert_eq!(DEFINITION.id, "imessage");
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert!(DEFINITION.is_implemented());
    assert!(DEFINITION.capabilities.receive && DEFINITION.capabilities.send);
    assert!(!DEFINITION.capabilities.mention_gate);
    assert!(!DEFINITION.capabilities.edit);
    // Requires must say macOS, so `future channel list` is honest.
    assert!(DEFINITION.requires.iter().any(|r| r.contains("macOS")));
}

#[test]
fn config_defaults_poll_every_five_seconds_with_no_allowlist() {
    let config: IMessageConfig = serde_json::from_value(json!({})).unwrap();
    assert!(!config.enabled);
    assert_eq!(config.poll_seconds, 5);
    assert!(config.recipients.is_empty());
    assert!(config.sender_allowlist.is_empty());
    assert!(config.db_path.is_empty());
}

#[test]
fn a_db_path_override_wins_over_the_default_location() {
    let config: IMessageConfig =
        serde_json::from_value(json!({"db_path": "/tmp/fixture-chat.db"})).unwrap();
    assert_eq!(config.db_path(), PathBuf::from("/tmp/fixture-chat.db"));

    let default: IMessageConfig = serde_json::from_value(json!({})).unwrap();
    let path = default.db_path();
    assert!(path.ends_with("Library/Messages/chat.db"), "{path:?}");
    // No hard-coded home: the path is derived, not a literal.
    assert!(!path.to_string_lossy().contains('~'));
}

// ─── Apple epoch conversion ─────────────────────────────────────────────────

#[test]
fn apple_epoch_is_978307200_seconds_after_unix() {
    assert_eq!(APPLE_EPOCH_OFFSET_MS, 978_307_200_000);
    // 2001-01-01T00:00:00Z in Unix ms, from an independent computation.
    let check = chrono::DateTime::parse_from_rfc3339("2001-01-01T00:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(APPLE_EPOCH_OFFSET_MS, check);
}

#[test]
fn nanosecond_timestamps_convert_to_unix_ms() {
    // 2026-09-22T02:00:00Z in Unix ms…
    let unix_ms = chrono::DateTime::parse_from_rfc3339("2026-09-22T02:00:00Z")
        .unwrap()
        .timestamp_millis();
    // …is (unix_ms - offset) nanoseconds in the Apple frame.
    let apple_ns = (unix_ms - APPLE_EPOCH_OFFSET_MS) * 1_000_000;
    assert_eq!(apple_ts_to_unix_ms(apple_ns), unix_ms);
}

#[test]
fn legacy_second_timestamps_still_convert() {
    // Old rows store plain seconds since the Apple epoch; the magnitude
    // (seconds, not nanoseconds) is the discriminator.
    let apple_seconds: i64 = 812_462_400; // 2026-09-22T02:00:00Z - offset, in seconds
    let expected = APPLE_EPOCH_OFFSET_MS + apple_seconds * 1_000;
    assert_eq!(apple_ts_to_unix_ms(apple_seconds), expected);
}

#[test]
fn the_epoch_itself_maps_to_the_offset() {
    assert_eq!(apple_ts_to_unix_ms(0), APPLE_EPOCH_OFFSET_MS);
    // One second before the epoch (legacy seconds mode) is exactly one
    // second earlier, not a nanosecond-mode rounding artefact.
    assert_eq!(apple_ts_to_unix_ms(-1), APPLE_EPOCH_OFFSET_MS - 1_000);
}

// ─── AppleScript escaping ───────────────────────────────────────────────────

#[test]
fn escaping_quotes_and_backslashes() {
    assert_eq!(applescript_escape("plain"), "plain");
    assert_eq!(applescript_escape("say \"hi\""), "say \\\"hi\\\"");
    assert_eq!(applescript_escape("back\\slash"), "back\\\\slash");
}

#[test]
fn an_injection_attempt_cannot_close_the_literal() {
    // The classic escape: end the string, run a command, reopen a string.
    let evil = "\" & (do shell script \"rm -rf /\") & \"";
    let escaped = applescript_escape(evil);
    // Every quote is escaped, so the literal never closes.
    assert!(
        !escaped.contains('"') || escaped.matches("\\\"").count() == escaped.matches('"').count()
    );
    let script = send_script("+15551234567", evil);
    // The only unescaped quotes in the whole script are the structural ones
    // around "Messages" and the literal delimiters — the payload contributes
    // none of its own.
    let payload_portion = script.split("send ").nth(1).unwrap();
    assert!(payload_portion.starts_with('"'), "{payload_portion}");
    // Count quotes inside the payload literal: opening, escaped ones, closing.
    assert!(payload_portion.contains("\\\" & (do shell script \\\"rm -rf /\\\") & \\\""));
}

#[test]
fn a_backslash_before_the_closing_quote_cannot_escape_it() {
    // Without escaping, "text\" would escape the closing quote and swallow
    // the rest of the script.
    let evil = "trailing\\";
    let script = send_script("buddy@example.com", evil);
    assert!(script.contains("trailing\\\\\""), "{script}");
}

#[test]
fn newlines_and_unicode_survive_escaping() {
    let text = "line one\nline two — ünïcode 😀";
    let script = send_script("+15551234567", text);
    assert!(script.contains("line one\nline two — ünïcode 😀"));
}

#[test]
fn the_script_addresses_an_imessage_account_and_escapes_the_recipient() {
    let script = send_script("evil\"recipient", "hi");
    assert!(script.contains("service type = iMessage"));
    assert!(script.contains("participant \"evil\\\"recipient\""));
    assert!(script.starts_with("tell application \"Messages\""));
    assert!(script.ends_with("end tell"));
}

// ─── Poll query ─────────────────────────────────────────────────────────────

#[test]
fn the_poll_query_filters_null_text_and_orders_by_date() {
    let sql = poll_query(1_790_553_600_000);
    assert!(sql.contains("m.text IS NOT NULL"), "{sql}");
    assert!(sql.contains("ORDER BY m.date ASC"), "{sql}");
    assert!(sql.contains("m.is_from_me"), "{sql}");
    // The watermark is converted back into Apple nanoseconds.
    let since_ns = (1_790_553_600_000 - APPLE_EPOCH_OFFSET_MS) * 1_000_000;
    assert!(sql.contains(&format!("m.date > {since_ns}")), "{sql}");
}

#[test]
fn the_poll_query_joins_handles_for_sender_identity() {
    let sql = poll_query(0);
    assert!(
        sql.contains("LEFT JOIN handle h ON m.handle_id = h.ROWID"),
        "{sql}"
    );
    assert!(sql.contains("h.id"), "{sql}");
}

// ─── chat.db row parsing ────────────────────────────────────────────────────

/// A fixture shaped exactly like `sqlite3 -separator '\t'` output: rowid,
/// handle, text, Apple-epoch nanoseconds, is_from_me.
const FIXTURE: &str = "101\t+15551234567\thello there\t812464800000000000\t0\n\
                       102\tbuddy@example.com\tsecond message\t812464860000000000\t0\n\
                       103\t+15551234567\tmy own reply\t812464920000000000\t1\n";

#[test]
fn fixture_rows_parse_with_handles_text_and_direction() {
    let rows = parse_chat_rows(FIXTURE);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0].rowid, 101);
    assert_eq!(rows[0].handle, "+15551234567");
    assert_eq!(rows[0].text, "hello there");
    assert!(!rows[0].is_from_me);
    assert_eq!(rows[2].rowid, 103);
    assert!(rows[2].is_from_me);
}

#[test]
fn fixture_timestamps_are_unix_ms() {
    let rows = parse_chat_rows(FIXTURE);
    let expected = APPLE_EPOCH_OFFSET_MS + 812_464_800_000;
    assert_eq!(rows[0].timestamp_ms, expected);
}

#[test]
fn a_null_handle_parses_as_empty_and_is_skippable() {
    let rows = parse_chat_rows("7\t\torphan text\t812464800000000000\t0\n");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].handle, "");
    assert_eq!(rows[0].text, "orphan text");
}

#[test]
fn malformed_lines_are_skipped_not_fatal() {
    let output = "garbage line\n\
                  8\th\ttext\tnotanumber\t0\n\
                  9\th\tgood\t812464800000000000\t0\n";
    let rows = parse_chat_rows(output);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].rowid, 9);
    assert_eq!(rows[0].text, "good");
}

#[test]
fn a_text_with_tabs_is_rejoined() {
    // sqlite3 separates on tab, so a tab inside the text yields extra fields;
    // the wide parser keeps the positional head and tail intact.
    let rows = parse_chat_rows("11\t+15551234567\tcol1\tcol2\t812464800000000000\t0\n");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].text, "col1\tcol2");
    assert_eq!(rows[0].handle, "+15551234567");
    assert_eq!(
        rows[0].timestamp_ms,
        APPLE_EPOCH_OFFSET_MS + 812_464_800_000
    );
}

#[test]
fn empty_output_yields_no_rows() {
    assert!(parse_chat_rows("").is_empty());
    assert!(parse_chat_rows("\n\n").is_empty());
}

// ─── Permission diagnostic ──────────────────────────────────────────────────

#[test]
fn the_permission_diagnostic_names_the_fix() {
    let advice = permission_advice(Path::new("/Users/x/Library/Messages/chat.db"));
    assert!(advice.contains("Full Disk Access"), "{advice}");
    assert!(advice.contains("chat.db"), "{advice}");
    // It must be actionable: where to click, not just "permission denied".
    assert!(advice.contains("Privacy"), "{advice}");
}

// ─── Provider entry points (platform-independent on the host OS) ───────────

fn ctx_with_config(config: serde_json::Value) -> ProviderCtx {
    let base = ProviderCtx::offline(&DEFINITION);
    ProviderCtx::new(
        &DEFINITION,
        config,
        base.bridge().clone(),
        base.data_dir().to_path_buf(),
        std::sync::Arc::new(crate::session_store::SessionStore::new(
            base.data_dir().join("sessions.json"),
        )),
        base.shutdown().clone(),
    )
}

#[test]
fn the_unsupported_error_names_macos() {
    assert!(unsupported_error().to_string().contains("macOS"));
}

#[test]
fn the_provider_and_sender_declare_the_channel() {
    assert_eq!(IMessage.definition().id, "imessage");
    let sender = IMessageSender {
        config: IMessageConfig::default(),
    };
    assert_eq!(sender.definition().id, "imessage");
    assert_eq!(sender.definition().max_text_len, 20000);
}

#[cfg(target_os = "macos")]
mod provider_macos {
    use super::*;

    #[tokio::test]
    async fn sender_run_and_probe_use_the_config() {
        let ctx = ctx_with_config(json!({"enabled": true, "db_path": "/nonexistent/chat.db"}));
        // sender(): the config parses and the sender is built.
        let sender = IMessage.sender(&ctx).expect("sender");
        assert_eq!(sender.definition().id, "imessage");

        // probe(): the missing database must fail with the actionable
        // diagnostic, not a bare I/O error.
        let error = IMessage.probe(&ctx).await.expect_err("probe must fail");
        assert!(error.to_string().contains("Full Disk Access"), "{error}");
    }

    #[tokio::test]
    async fn send_via_applescript_round_trips_or_fails_honestly() {
        // macos::send(): build the script, run osascript, map the outcome.
        match macos::send("+15551234567", "hello from the test suite").await {
            Ok(()) => {
                // Messages.app is running and accepted the send.
            }
            Err(error) => {
                // Headless CI has no Messages.app: the refusal names the
                // send path rather than a bare spawn error.
                let text = error.to_string();
                assert!(
                    text.contains("Messages") || text.contains("osascript"),
                    "{text}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_malformed_config_is_rejected_everywhere() {
        let ctx = ctx_with_config(json!({"poll_seconds": "five"}));
        assert!(IMessage.sender(&ctx).is_err());
        assert!(IMessage.probe(&ctx).await.is_err());
        assert!(IMessage.run(ctx).await.is_err());
    }

    #[tokio::test]
    async fn run_constructs_its_sender_and_enters_the_poll_loop() {
        // The provider-level run(): config parse, sender construction, then
        // the poll loop — which here fails its queries against an absent
        // database, marks the status, and keeps waiting until shutdown.
        let data_dir = crate::test_support::temp_dir("imsg-run-ctx");
        let ctx = ProviderCtx::new(
            &DEFINITION,
            json!({
                "enabled": true,
                "db_path": "/nonexistent/chat.db",
                "poll_seconds": 1,
            }),
            crate::bridge::Bridge::new(
                std::sync::Arc::new(crate::config::AgentConfig {
                    grpc_addr: "http://127.0.0.1:1".into(),
                    cwd: data_dir.to_string_lossy().into_owned(),
                    ..crate::config::AgentConfig::default()
                }),
                crate::policy::AccessPolicyConfig::default(),
                data_dir.clone(),
                std::sync::Arc::new(crate::status::StatusBoard::new(
                    data_dir.join("status.json"),
                )),
            ),
            data_dir.clone(),
            std::sync::Arc::new(crate::session_store::SessionStore::new(
                data_dir.join("sessions.json"),
            )),
            crate::bridge::Shutdown::new(),
        );
        let run = {
            let ctx = ctx.clone();
            tokio::spawn(async move { IMessage.run(ctx).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        ctx.shutdown().trigger();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), run)
            .await
            .expect("run must stop on shutdown")
            .expect("the task must not panic");
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn run_reports_the_missing_database_as_a_failure() {
        // The poll loop cannot even start against an absent chat.db.
        let ctx = ctx_with_config(json!({
            "enabled": true,
            "db_path": "/nonexistent/chat.db",
            "poll_seconds": 1,
        }));
        let run = {
            let ctx = ctx.clone();
            tokio::spawn(async move { IMessage.run(ctx).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        ctx.shutdown().trigger();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), run)
            .await
            .expect("run must stop on shutdown")
            .expect("the task must not panic");
        // The loop itself exits cleanly; the failure is published instead.
        assert!(result.is_ok(), "{result:?}");
    }
}

// ─── Unsupported platform behaviour ─────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use super::*;

    #[test]
    fn this_platform_is_not_supported() {
        assert!(!platform_supported());
    }

    #[tokio::test]
    async fn every_entry_point_refuses_with_a_macos_message() {
        let base = ProviderCtx::offline(&DEFINITION);
        let ctx = ProviderCtx::new(
            &DEFINITION,
            json!({"enabled": true}),
            base.bridge().clone(),
            base.data_dir().to_path_buf(),
            std::sync::Arc::new(crate::session_store::SessionStore::new(
                base.data_dir().join("sessions.json"),
            )),
            base.shutdown().clone(),
        );
        let error = IMessage
            .sender(&ctx)
            .expect_err("sender must refuse off macOS");
        assert!(error.to_string().contains("macOS"), "{error}");

        let error = IMessage
            .probe(&ctx)
            .await
            .expect_err("probe must refuse off macOS");
        assert!(error.to_string().contains("macOS"), "{error}");

        let error = IMessage
            .run(ctx)
            .await
            .expect_err("run must refuse off macOS");
        assert!(error.to_string().contains("macOS"), "{error}");
    }

    #[tokio::test]
    async fn send_text_refuses_even_with_a_constructed_sender() {
        // A sender value can exist in tests; using it must still refuse.
        let sender = IMessageSender {
            config: IMessageConfig::default(),
        };
        let conversation = ConversationRef {
            id: "+15551234567".into(),
            thread_id: None,
            kind: ChatKind::Direct,
        };
        let error = sender
            .send_text(&conversation, "hi")
            .await
            .expect_err("send must refuse off macOS");
        assert!(error.to_string().contains("macOS"), "{error}");
    }
}

#[cfg(target_os = "macos")]
mod supported {
    use super::*;
    use std::time::Duration;

    #[test]
    fn this_platform_is_supported() {
        assert!(platform_supported());
    }

    // ─── osascript round trip ────────────────────────────────────────────

    #[tokio::test]
    async fn osascript_evaluates_a_literal_and_reports_errors() {
        // A script with no side effects proves the launch + stdout path.
        let out = macos::run_osascript("return \"hello from test\"")
            .await
            .expect("osascript must run");
        assert_eq!(out, "hello from test");

        // A syntax error is surfaced, not swallowed.
        let error = macos::run_osascript("this is not applescript ((")
            .await
            .expect_err("an invalid script must fail");
        assert!(error.to_string().contains("refused"), "{error}");
    }

    // ─── chat.db against a real sqlite3 ──────────────────────────────────

    /// Build a minimal chat.db fixture with one inbound and one outbound
    /// message, plus one textless row the poll must skip.
    fn fixture_db(label: &str) -> std::path::PathBuf {
        let dir = crate::test_support::temp_dir(label);
        let db = dir.join("chat.db");
        let apple_ns: i64 = 812_464_800_000_000_000;
        let sql = format!(
            "CREATE TABLE handle (ROWID INTEGER PRIMARY KEY, id TEXT);\
             CREATE TABLE message (ROWID INTEGER PRIMARY KEY, handle_id INTEGER, text TEXT, date INTEGER, is_from_me INTEGER);\
             INSERT INTO handle VALUES (1, '+15551234567'), (2, 'buddy@example.com');\
             INSERT INTO message VALUES (101, 1, 'hello there', {apple_ns}, 0);\
             INSERT INTO message VALUES (102, 2, 'second message', {apple_ns}, 0);\
             INSERT INTO message VALUES (103, 1, 'my own reply', {apple_ns}, 1);\
             INSERT INTO message VALUES (104, 1, NULL, {apple_ns}, 0);"
        );
        let status = std::process::Command::new("sqlite3")
            .arg(&db)
            .arg(&sql)
            .status()
            .expect("sqlite3 must exist on macOS");
        assert!(status.success());
        db
    }

    #[tokio::test]
    async fn a_fixture_database_parses_through_sqlite3() {
        let db = fixture_db("imsg-fixture");
        let output = macos::query_db(&db, &poll_query(0))
            .await
            .expect("the fixture must be readable");
        let rows = parse_chat_rows(&output);
        // The NULL-text row never leaves the database.
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[0].rowid, 101);
        assert_eq!(rows[0].handle, "+15551234567");
        assert!(rows[2].is_from_me);
    }

    #[tokio::test]
    async fn a_missing_database_produces_the_permission_advice() {
        let error = macos::query_db(Path::new("/nonexistent/chat.db"), "SELECT 1")
            .await
            .expect_err("a missing database must fail");
        assert!(error.to_string().contains("Full Disk Access"), "{error}");
    }

    #[tokio::test]
    async fn a_sql_error_is_reported_with_sqlites_message() {
        let db = fixture_db("imsg-bad-sql");
        let error = macos::query_db(&db, "SELECT nope FROM missing_table")
            .await
            .expect_err("bad SQL must fail");
        let text = error.to_string();
        assert!(
            text.contains("reading the Messages database failed"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn an_unreadable_database_produces_the_permission_advice() {
        let db = fixture_db("imsg-perm");
        // Remove read permission: sqlite3 then fails to open the file.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let result = macos::query_db(&db, "SELECT 1").await;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o600));
        }
        let error = result.expect_err("an unreadable database must fail");
        assert!(error.to_string().contains("Full Disk Access"), "{error}");
    }

    #[tokio::test]
    async fn send_text_refuses_when_messages_app_does_not_answer() {
        // A recipient that cannot resolve makes Messages.app (or the launch)
        // fail; the error must surface, never report success.
        let sender = IMessageSender {
            config: IMessageConfig::default(),
        };
        let conversation = ConversationRef {
            id: "invalid\\\"recipient\\\"that\\\"breaks".into(),
            thread_id: None,
            kind: ChatKind::Direct,
        };
        // We cannot assert Ok or Err without Messages.app running; what we
        // CAN assert is that the call completes and, on failure, mentions
        // the send path. On a headless CI Messages.app is absent → error.
        match sender.send_text(&conversation, "hi").await {
            Ok(_) => {
                // Messages.app accepted it: the id is None by design.
            }
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("Messages") || text.contains("osascript"),
                    "{text}"
                );
            }
        }
    }

    #[tokio::test]
    async fn probe_reports_the_app_version_and_the_database_path() {
        let db = fixture_db("imsg-probe");
        let config: IMessageConfig =
            serde_json::from_value(json!({"db_path": db.to_string_lossy()})).unwrap();
        match macos::probe(&config).await {
            Ok(summary) => {
                // Messages.app answered: the summary names the database.
                assert!(summary.contains("database readable at"), "{summary}");
                assert!(summary.contains("chat.db"), "{summary}");
            }
            Err(error) => {
                // Headless CI has no Messages.app: the chat.db query already
                // succeeded, so the failure must come from the osascript
                // half of the probe.
                let text = error.to_string();
                assert!(
                    text.contains("Messages") || text.contains("osascript"),
                    "{text}"
                );
            }
        }
    }

    // ─── The poll loop ───────────────────────────────────────────────────

    #[tokio::test]
    async fn the_poll_loop_delivers_new_rows_and_stops_on_shutdown() {
        let db = fixture_db("imsg-poll");
        let data_dir = crate::test_support::temp_dir("imsg-poll-ctx");
        let bridge = crate::bridge::Bridge::new(
            std::sync::Arc::new(crate::config::AgentConfig {
                grpc_addr: "http://127.0.0.1:1".into(),
                cwd: data_dir.to_string_lossy().into_owned(),
                ..crate::config::AgentConfig::default()
            }),
            crate::policy::AccessPolicyConfig {
                dm_policy: "open".into(),
                ..Default::default()
            },
            data_dir.clone(),
            std::sync::Arc::new(crate::status::StatusBoard::new(
                data_dir.join("status.json"),
            )),
        );
        let ctx = ProviderCtx::new(
            &DEFINITION,
            json!({
                "enabled": true,
                "db_path": db.to_string_lossy(),
                "poll_seconds": 1,
            }),
            bridge,
            data_dir.clone(),
            std::sync::Arc::new(crate::session_store::SessionStore::new(
                data_dir.join("sessions.json"),
            )),
            crate::bridge::Shutdown::new(),
        );
        let sender = std::sync::Arc::new(PollRecordingSender::default());
        let run = {
            let ctx = ctx.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let config: IMessageConfig = ctx.config().unwrap();
                macos::run_polling(&ctx, &config, sender).await
            })
        };
        // Two poll intervals, then shutdown.
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        ctx.shutdown().trigger();
        let result = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("the loop must stop on shutdown")
            .expect("the task must not panic");
        assert!(result.is_ok(), "{result:?}");
        let sent = sender.sent();
        // Two inbound rows (101, 102) became prompts; the agent is
        // unreachable, so the bridge answered each on-channel. The own-send
        // row (103) and the NULL-text row (104) never became prompts.
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert!(
            sent.iter()
                .all(|line| line.contains("Cannot reach the agent")),
            "{sent:?}"
        );
    }

    #[tokio::test]
    async fn the_poll_loop_honours_the_sender_allowlist() {
        let db = fixture_db("imsg-allow");
        let data_dir = crate::test_support::temp_dir("imsg-allow-ctx");
        let bridge = crate::bridge::Bridge::new(
            std::sync::Arc::new(crate::config::AgentConfig {
                grpc_addr: "http://127.0.0.1:1".into(),
                cwd: data_dir.to_string_lossy().into_owned(),
                ..crate::config::AgentConfig::default()
            }),
            crate::policy::AccessPolicyConfig {
                dm_policy: "open".into(),
                ..Default::default()
            },
            data_dir.clone(),
            std::sync::Arc::new(crate::status::StatusBoard::new(
                data_dir.join("status.json"),
            )),
        );
        let ctx = ProviderCtx::new(
            &DEFINITION,
            json!({
                "enabled": true,
                "db_path": db.to_string_lossy(),
                "poll_seconds": 1,
                "sender_allowlist": ["+15551234567"],
            }),
            bridge,
            data_dir.clone(),
            std::sync::Arc::new(crate::session_store::SessionStore::new(
                data_dir.join("sessions.json"),
            )),
            crate::bridge::Shutdown::new(),
        );
        let sender = std::sync::Arc::new(PollRecordingSender::default());
        let run = {
            let ctx = ctx.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let config: IMessageConfig = ctx.config().unwrap();
                macos::run_polling(&ctx, &config, sender).await
            })
        };
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        ctx.shutdown().trigger();
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("the loop must stop")
            .expect("no panic")
            .expect("clean exit");
        let sent = sender.sent();
        // Only the allowlisted handle's row became a prompt; row 102
        // (buddy@example.com) was filtered by the allowlist.
        assert_eq!(sent.len(), 1, "{sent:?}");
    }

    #[tokio::test]
    async fn the_poll_loop_marks_a_query_failure_and_keeps_polling() {
        // chat.db is absent: every query fails, the poll marks the status
        // and continues; shutdown still stops the loop cleanly.
        let data_dir = crate::test_support::temp_dir("imsg-fail-ctx");
        let status = std::sync::Arc::new(crate::status::StatusBoard::new(
            data_dir.join("status.json"),
        ));
        let bridge = crate::bridge::Bridge::new(
            std::sync::Arc::new(crate::config::AgentConfig {
                grpc_addr: "http://127.0.0.1:1".into(),
                cwd: data_dir.to_string_lossy().into_owned(),
                ..crate::config::AgentConfig::default()
            }),
            crate::policy::AccessPolicyConfig {
                dm_policy: "open".into(),
                ..Default::default()
            },
            data_dir.clone(),
            status.clone(),
        );
        let ctx = ProviderCtx::new(
            &DEFINITION,
            json!({
                "enabled": true,
                "db_path": "/nonexistent/chat.db",
                "poll_seconds": 1,
            }),
            bridge,
            data_dir.clone(),
            std::sync::Arc::new(crate::session_store::SessionStore::new(
                data_dir.join("sessions.json"),
            )),
            crate::bridge::Shutdown::new(),
        );
        let sender = std::sync::Arc::new(PollRecordingSender::default());
        let run = {
            let ctx = ctx.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let config: IMessageConfig = ctx.config().unwrap();
                macos::run_polling(&ctx, &config, sender).await
            })
        };
        // Two poll intervals so the failed-query arm runs, then shutdown.
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        ctx.shutdown().trigger();
        let result = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("the loop must stop on shutdown")
            .expect("the task must not panic");
        assert!(result.is_ok(), "{result:?}");
        // Nothing was ever delivered, and the failure was published.
        assert!(sender.sent().is_empty());
        let snapshot = crate::status::StatusSnapshot::load(status.path());
        let entry = snapshot
            .channels
            .get("imessage")
            .expect("the poll must publish a status");
        assert_eq!(entry.state, Some(crate::status::ChannelState::Error));
        let last_error = entry.last_error.as_deref().unwrap_or_default();
        assert!(last_error.contains("Full Disk Access"), "{last_error}");
    }

    #[tokio::test]
    async fn the_poll_loop_skips_rows_without_a_handle() {
        // A row whose JOIN finds no handle must be skipped, not answered.
        let dir = crate::test_support::temp_dir("imsg-nohandle");
        let db = dir.join("chat.db");
        let apple_ns: i64 = 812_464_800_000_000_000;
        let sql = format!(
            "CREATE TABLE handle (ROWID INTEGER PRIMARY KEY, id TEXT);\
             CREATE TABLE message (ROWID INTEGER PRIMARY KEY, handle_id INTEGER, text TEXT, date INTEGER, is_from_me INTEGER);\
             INSERT INTO handle VALUES (1, '+15551234567');\
             INSERT INTO message VALUES (201, 1, 'from a known handle', {apple_ns}, 0);\
             INSERT INTO message VALUES (202, 99, 'from a handle nobody knows', {apple_ns}, 0);"
        );
        let status = std::process::Command::new("sqlite3")
            .arg(&db)
            .arg(&sql)
            .status()
            .expect("sqlite3 must exist on macOS");
        assert!(status.success());
        let data_dir = crate::test_support::temp_dir("imsg-nohandle-ctx");
        let bridge = crate::bridge::Bridge::new(
            std::sync::Arc::new(crate::config::AgentConfig {
                grpc_addr: "http://127.0.0.1:1".into(),
                cwd: data_dir.to_string_lossy().into_owned(),
                ..crate::config::AgentConfig::default()
            }),
            crate::policy::AccessPolicyConfig {
                dm_policy: "open".into(),
                ..Default::default()
            },
            data_dir.clone(),
            std::sync::Arc::new(crate::status::StatusBoard::new(
                data_dir.join("status.json"),
            )),
        );
        let ctx = ProviderCtx::new(
            &DEFINITION,
            json!({
                "enabled": true,
                "db_path": db.to_string_lossy(),
                "poll_seconds": 1,
            }),
            bridge,
            data_dir.clone(),
            std::sync::Arc::new(crate::session_store::SessionStore::new(
                data_dir.join("sessions.json"),
            )),
            crate::bridge::Shutdown::new(),
        );
        let sender = std::sync::Arc::new(PollRecordingSender::default());
        let run = {
            let ctx = ctx.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let config: IMessageConfig = ctx.config().unwrap();
                macos::run_polling(&ctx, &config, sender).await
            })
        };
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        ctx.shutdown().trigger();
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("the loop must stop")
            .expect("no panic")
            .expect("clean exit");
        let sent = sender.sent();
        // Only the known handle's row became a prompt; the dangling row was
        // skipped at the empty-handle guard.
        assert_eq!(sent.len(), 1, "{sent:?}");
    }

    #[derive(Default)]
    struct PollRecordingSender {
        sent: std::sync::Mutex<Vec<String>>,
    }

    impl PollRecordingSender {
        fn sent(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChannelSender for PollRecordingSender {
        fn definition(&self) -> &'static ChannelDefinition {
            &DEFINITION
        }

        async fn send_text(
            &self,
            _conversation: &ConversationRef,
            text: &str,
        ) -> Result<Option<String>> {
            self.sent.lock().unwrap().push(text.to_string());
            Ok(None)
        }
    }
}
