//! Tests for the IRC provider: line parsing, the byte cap, addressing, and the
//! registration/keepalive loop against a local socket that speaks the protocol.
//!
//! Nothing here reaches a network: the server side of each protocol test is a
//! plain TCP listener in this file.

use super::*;
use crate::test_support::{temp_dir, wait_until};
use serde_json::Value;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ─── helpers ───────────────────────────────────────────────────────────────

/// A local stand-in for an IRC server.
struct MockIrc {
    addr: SocketAddr,
    /// Everything the client wrote, across all connections.
    received: Arc<Mutex<Vec<String>>>,
    /// Lines to hand the client after the script.
    out: mpsc::UnboundedSender<String>,
    connections: Arc<std::sync::atomic::AtomicUsize>,
    /// Asks a TLS server to end the current connection. The plaintext mock has
    /// no receiver, so sending into it is a no-op.
    closer: mpsc::UnboundedSender<Close>,
}

impl MockIrc {
    fn lines(&self) -> Vec<String> {
        self.received.lock().clone()
    }

    fn connections(&self) -> usize {
        self.connections.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn sent(&self, line: &str) {
        self.out.send(line.to_string()).ok();
    }

    /// End the current TLS connection in the given way, so the client's
    /// reaction to each kind of closure can be told apart.
    fn close(&self, how: Close) {
        self.closer.send(how).ok();
    }

    fn lines_matching(&self, needle: &str) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(needle))
            .collect()
    }

    async fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        wait_until(
            || self.lines().iter().any(|line| line.contains(needle)),
            timeout,
        )
        .await
    }
}

/// A listener answering every connection with `script`, then optionally
/// closing it (which is how a dropped connection is simulated).
async fn spawn_irc(script: Vec<String>, keep_open: bool) -> MockIrc {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (out, out_rx) = mpsc::unbounded_channel::<String>();
    let out_rx = Arc::new(tokio::sync::Mutex::new(out_rx));
    let received_task = received.clone();
    let connections_task = connections.clone();

    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            connections_task.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let script = script.clone();
            let received = received_task.clone();
            let out_rx = out_rx.clone();
            tokio::spawn(async move {
                let (mut read, mut write) = tokio::io::split(socket);
                let writer = tokio::spawn(async move {
                    for line in script {
                        if write
                            .write_all(format!("{line}\r\n").as_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    if !keep_open {
                        // Dropping one half of a split stream does not close the
                        // socket; an explicit shutdown is what the client sees
                        // as a dropped connection.
                        let _ = write.shutdown().await;
                        return;
                    }
                    let mut out_rx = out_rx.lock().await;
                    while let Some(line) = out_rx.recv().await {
                        if write
                            .write_all(format!("{line}\r\n").as_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });

                let mut buf = [0u8; 4096];
                let mut pending: Vec<u8> = Vec::new();
                loop {
                    match read.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            pending.extend_from_slice(&buf[..read]);
                            while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                                let line: Vec<u8> = pending.drain(..=index).collect();
                                let line = &line[..line.len() - 1];
                                let line = line.strip_suffix(b"\r").unwrap_or(line);
                                received
                                    .lock()
                                    .push(String::from_utf8_lossy(line).into_owned());
                            }
                        }
                    }
                }
                if !keep_open {
                    writer.abort();
                }
            });
        }
    });

    MockIrc {
        addr,
        received,
        out,
        connections,
        // A plaintext server ends a connection when its script says so.
        closer: mpsc::unbounded_channel().0,
    }
}

/// A listener that writes `payload` verbatim — no line terminator appended —
/// which is how a server that is not speaking IRC looks from the client side.
async fn spawn_irc_raw(payload: &[u8], keep_open: bool) -> MockIrc {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (out, out_rx) = mpsc::unbounded_channel::<String>();
    let out_rx = Arc::new(tokio::sync::Mutex::new(out_rx));
    let payload = payload.to_vec();
    let connections_task = connections.clone();

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            connections_task.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let payload = payload.clone();
            let out_rx = out_rx.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                if socket.write_all(&payload).await.is_err() {
                    return;
                }
                if !keep_open {
                    // Drain what the client sent first: a socket closed with
                    // unread data in it answers with a reset, and the client
                    // must see an orderly end of stream instead.
                    let mut sink = [0u8; 4096];
                    let _ = socket.read(&mut sink).await;
                    let _ = socket.shutdown().await;
                    return;
                }
                // Keep the connection up: the client must decide on its own
                // that what it received is not a message.
                let mut sink = [0u8; 1024];
                let mut out_rx = out_rx.lock().await;
                while out_rx.recv().await.is_some() {
                    if socket.read(&mut sink).await.is_err() {
                        return;
                    }
                }
            });
        }
    });

    MockIrc {
        addr,
        received,
        out,
        connections,
        closer: mpsc::unbounded_channel().0,
    }
}

/// The `providers.irc` block pointing at a local listener.
fn config_for(server: &MockIrc, extra: Value) -> Value {
    let mut block = serde_json::json!({
        "enabled": true,
        "server": "127.0.0.1",
        "port": server.addr.port(),
        "tls": false,
        "nick": "bot",
        "channels": ["#future"],
        "idle_ping_seconds": 0,
    });
    if let (Some(block), Some(extra)) = (block.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            block.insert(key.clone(), value.clone());
        }
    }
    block
}

/// A context over an offline bridge with an open policy, so an accepted message
/// produces exactly one outbound reply (the agent is unreachable in tests) and
/// counting replies is deterministic.
fn ctx_with(block: Value) -> ProviderCtx {
    let data_dir = temp_dir("irc-ctx");
    let sessions = Arc::new(crate::session_store::SessionStore::new(
        data_dir.join("sessions.json"),
    ));
    let agent_cfg = crate::config::AgentConfig {
        grpc_addr: "http://127.0.0.1:1".into(),
        cwd: data_dir.to_string_lossy().into_owned(),
        ..crate::config::AgentConfig::default()
    };
    let bridge = crate::bridge::Bridge::new(
        Arc::new(agent_cfg),
        crate::policy::AccessPolicyConfig {
            dm_policy: "open".into(),
            group_policy: "open".into(),
            require_mention: false,
            ..crate::policy::AccessPolicyConfig::default()
        },
        data_dir.clone(),
        Arc::new(crate::status::StatusBoard::new(
            data_dir.join("status.json"),
        )),
    );
    ProviderCtx::new(
        &DEFINITION,
        block,
        bridge,
        data_dir,
        sessions,
        Arc::new(tokio::sync::Notify::new()),
    )
}

/// Run the provider in the background and give back its handle.
fn spawn_run(ctx: &ProviderCtx) -> tokio::task::JoinHandle<Result<()>> {
    let ctx = ctx.clone();
    tokio::spawn(async move { Irc::new().run(ctx).await })
}

// ─── line parsing ──────────────────────────────────────────────────────────

#[test]
fn a_plain_line_splits_into_command_and_params() {
    let msg = parse_line("PRIVMSG #future :hello there\r\n").expect("parsed");
    assert_eq!(msg.command, "PRIVMSG");
    assert_eq!(msg.params, vec!["#future", "hello there"]);
    assert!(msg.prefix.is_none());
    assert!(msg.tags.is_empty());
}

#[test]
fn a_prefix_is_kept_and_the_nickname_can_be_extracted() {
    let msg = parse_line(":alice!user@host.example PRIVMSG #future :hi").expect("parsed");
    assert_eq!(msg.prefix.as_deref(), Some("alice!user@host.example"));
    assert_eq!(msg.nickname(), Some("alice"));
    // A server name is a prefix too, with no nickname in it.
    let server = parse_line(":irc.example 001 bot :Welcome").expect("parsed");
    assert_eq!(server.nickname(), Some("irc.example"));
}

#[test]
fn the_trailing_parameter_keeps_its_spaces_and_colons() {
    let msg = parse_line(":srv NOTICE bot :a: b  c").expect("parsed");
    assert_eq!(msg.params, vec!["bot", "a: b  c"]);
    let multi = parse_line(":srv 353 bot = #future :a b c").expect("parsed");
    assert_eq!(multi.params, vec!["bot", "=", "#future", "a b c"]);
}

#[test]
fn ircv3_tags_are_parsed_and_unescaped() {
    let msg = parse_line("@time=2024-01-02T03:04:05.678Z;flag :alice!u@h PRIVMSG #c :hi")
        .expect("parsed");
    assert_eq!(msg.tag("time"), Some("2024-01-02T03:04:05.678Z"));
    assert_eq!(msg.tag("flag"), Some(""));
    assert_eq!(msg.tag("missing"), None);
    let escaped = parse_line("@key=a\\sb\\:c :srv PING :x").expect("parsed");
    assert_eq!(escaped.tag("key"), Some("a b;c"));
}

#[test]
fn lines_without_a_command_are_not_messages() {
    assert!(parse_line("").is_none());
    assert!(parse_line("\r\n").is_none());
    assert!(parse_line("   ").is_none());
}

#[test]
fn a_server_timestamp_becomes_a_unix_millisecond_stamp() {
    let msg = parse_line("@time=1970-01-01T00:00:01.500Z :srv PING :x").expect("parsed");
    assert_eq!(server_time_ms(&msg.tags), Some(1_500));
    // Without the tag there is no timestamp, and inventing one would defeat the
    // bridge's replay filter.
    assert_eq!(server_time_ms(&[]), None);
    let bad = parse_line("@time=yesterday :srv PING :x").expect("parsed");
    assert_eq!(server_time_ms(&bad.tags), None);
}

#[test]
fn a_command_without_parameters_or_a_trailing_block_is_still_a_message() {
    // `PING` on its own has a command and no parameters at all.
    let bare = parse_line("PING").expect("parsed");
    assert_eq!(bare.command, "PING");
    assert!(bare.params.is_empty());
    assert_eq!(bare.trailing(), "");

    // A tag block with no space after it leaves nothing to parse.
    assert!(parse_line("@flag").is_none());
    // A prefix with no space after it does too.
    assert!(parse_line(":nick!u@h").is_none());
    // Tags with no command beyond them are not messages either.
    assert!(parse_line("@flag=1").is_none());
}

#[test]
fn tag_escapes_are_decoded() {
    // Raw string: the backslashes are the wire format, not Rust escapes.
    // `\s` is a space, `\:` a semicolon, `\r` and `\n` are themselves, and
    // any other escape is the character that follows it.
    let msg = parse_line(r"@k=a\sb\:c\rd\ne\qf :srv PING :token").expect("parsed");
    // a, `\s`→space, b, `\:`→;, c, `\r`→CR, d, `\n`→LF, e, `\q`→q, f
    assert_eq!(msg.tag("k"), Some("a b;c\rd\neqf"));
    // The escapes belong to the tag block and leave the rest of the line alone.
    assert_eq!(msg.prefix.as_deref(), Some("srv"));
    assert_eq!(msg.command, "PING");
    assert_eq!(msg.trailing(), "token");
    // A lone trailing backslash has nothing to escape.
    let trailing = parse_line(r"@k=tail\ :srv PING :x").expect("parsed");
    assert_eq!(trailing.tag("k"), Some("tail\\"));
    // The block ends at the first real space, so the value can hold `:` freely.
    assert_eq!(trailing.command, "PING");
}

// ─── inbound normalization ─────────────────────────────────────────────────

fn message(line: &str) -> Message {
    parse_line(line).expect("parsed")
}

fn irc_config() -> IrcConfig {
    IrcConfig {
        server: "irc.example".into(),
        nick: "bot".into(),
        channels: vec!["#future".into()],
        ..IrcConfig::default()
    }
}

#[test]
fn a_channel_mention_becomes_an_addressed_message() {
    let inbound = to_inbound(
        &message(":alice!u@h PRIVMSG #future :bot: summarise this"),
        "bot",
        &irc_config(),
        7,
    )
    .expect("inbound");
    assert_eq!(inbound.conversation.kind, ChatKind::Channel);
    assert_eq!(inbound.conversation.id, "#future");
    assert_eq!(inbound.sender.id, "alice");
    assert_eq!(inbound.sender.display.as_deref(), Some("alice!u@h"));
    assert!(inbound.addressed_to_bot);
    // The address itself is not part of the prompt.
    assert_eq!(inbound.text, "summarise this");
    assert_eq!(inbound.message_id, "irc-7");
}

#[test]
fn a_channel_message_without_the_nickname_is_not_addressed() {
    for line in [
        ":alice!u@h PRIVMSG #future :what is going on?",
        // A nick that merely starts with the bot's is not an address.
        ":alice!u@h PRIVMSG #future :bother: hi",
        ":alice!u@h PRIVMSG #future :robot: hi",
    ] {
        let inbound = to_inbound(&message(line), "bot", &irc_config(), 1).expect("inbound");
        assert!(!inbound.addressed_to_bot, "{line}");
    }
}

#[test]
fn the_comma_and_at_conventions_also_address_the_bot() {
    for (line, expected) in [
        (":a!u@h PRIVMSG #future :bot, ping", "ping"),
        (":a!u@h PRIVMSG #future :@bot ping", "ping"),
        (":a!u@h PRIVMSG #future :BOT: Ping", "Ping"),
        (":a!u@h PRIVMSG #future :bot", ""),
    ] {
        let inbound = to_inbound(&message(line), "bot", &irc_config(), 1).expect("inbound");
        assert!(inbound.addressed_to_bot, "{line}");
        assert_eq!(inbound.text, expected, "{line}");
    }
}

#[test]
fn a_direct_message_is_always_addressed() {
    let inbound = to_inbound(
        &message(":alice!u@h PRIVMSG bot :hello there"),
        "bot",
        &irc_config(),
        3,
    )
    .expect("inbound");
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert_eq!(inbound.conversation.id, "alice");
    assert!(inbound.addressed_to_bot);
    assert_eq!(inbound.text, "hello there");
}

#[test]
fn messages_in_channels_this_bridge_does_not_answer_in_are_dropped() {
    let config = irc_config();
    assert!(to_inbound(
        &message(":alice!u@h PRIVMSG #other :bot: hello"),
        "bot",
        &config,
        1
    )
    .is_none());
    // Case does not matter: channels are compared case-insensitively.
    assert!(to_inbound(
        &message(":alice!u@h PRIVMSG #FUTURE :bot: hello"),
        "bot",
        &config,
        1
    )
    .is_some());
}

#[test]
fn ctcp_and_our_own_echoes_are_not_prompts() {
    let config = irc_config();
    // CTCP ACTION is what /me produces; a VERSION probe is not for the model
    // either.
    assert!(to_inbound(
        &message(":alice!u@h PRIVMSG #future :\u{1}ACTION waves\u{1}"),
        "bot",
        &config,
        1
    )
    .is_none());
    assert!(to_inbound(
        &message(":alice!u@h PRIVMSG bot :\u{1}VERSION\u{1}"),
        "bot",
        &config,
        1
    )
    .is_none());
    // Our own message coming back from the server must not be answered.
    assert!(to_inbound(
        &message(":bot!u@h PRIVMSG #future :bot: hello"),
        "bot",
        &config,
        1
    )
    .is_none());
}

#[test]
fn a_privmsg_without_a_prefix_or_text_is_dropped() {
    let config = irc_config();
    assert!(to_inbound(&message("PRIVMSG #future :hi"), "bot", &config, 1).is_none());
    assert!(to_inbound(&message(":a!u@h PRIVMSG #future"), "bot", &config, 1).is_none());
    assert!(to_inbound(&message(":a!u@h JOIN #future"), "bot", &config, 1).is_none());
}

#[test]
fn a_renamed_bot_still_recognises_its_own_nickname() {
    // After a collision the session nick is `bot_`, and that is what a user
    // sees and types.
    let inbound = to_inbound(
        &message(":alice!u@h PRIVMSG #future :bot_: hello"),
        "bot_",
        &irc_config(),
        1,
    )
    .expect("inbound");
    assert!(inbound.addressed_to_bot);
    assert_eq!(inbound.text, "hello");
}

// ─── outbound shaping ──────────────────────────────────────────────────────

#[test]
fn every_privmsg_line_stays_inside_the_512_byte_cap() {
    let target = "#a-fairly-long-channel-name";
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(60);
    let lines = split_privmsg(target, &text).expect("split");
    assert!(lines.len() > 1, "the text must not fit in one line");
    for line in &lines {
        assert!(
            line.len() + 2 <= MAX_LINE_BYTES,
            "{} bytes is over the cap: {line}",
            line.len() + 2
        );
        assert!(line.starts_with(&format!("PRIVMSG {target} :")), "{line}");
    }
    // Nothing is lost in the split.
    let rejoined: String = lines
        .iter()
        .map(|line| line.split_once(" :").expect("trailer").1)
        .collect();
    assert_eq!(rejoined.len(), text.len());
}

#[test]
fn the_budget_shrinks_as_the_target_grows() {
    assert_eq!(
        privmsg_budget("#c"),
        MAX_LINE_BYTES - 2 - PRIVMSG_OVERHEAD - "#c".len() - 2
    );
    assert!(privmsg_budget("#a-long-channel") < privmsg_budget("#c"));
    // An absurd target must not silently produce an unusable line.
    let error = split_privmsg(&"#".repeat(600), "hello").expect_err("must refuse");
    assert!(error.to_string().contains("no room"), "{error}");
}

#[test]
fn a_multibyte_text_is_split_on_character_boundaries() {
    let target = "#f";
    // Every character is three bytes, so a byte-counted split would cut one in
    // half and produce invalid UTF-8.
    let lines = split_privmsg(target, &"日本語".repeat(200)).expect("split");
    assert!(lines.len() > 1);
    for line in &lines {
        assert!(line.len() + 2 <= MAX_LINE_BYTES, "{}", line.len() + 2);
        let piece = line.split_once(" :").expect("trailer").1;
        assert!(!piece.is_empty());
        assert!(piece
            .chars()
            .all(|ch| ch == '日' || ch == '本' || ch == '語'));
    }
}

#[test]
fn control_characters_cannot_inject_a_protocol_command() {
    let lines = split_privmsg("#future", "hello\r\nQUIT :bye\u{1}").expect("split");
    // One line, and the newline is gone: a reply cannot become a command.
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0], "PRIVMSG #future :hello  QUIT :bye ");
    assert!(!lines[0].contains('\r') && !lines[0].contains('\n'));
    assert_eq!(sanitize("a\0b\tc"), "a b c");
}

#[test]
fn a_message_that_is_only_whitespace_sends_nothing() {
    assert!(split_privmsg("#future", "   \n ")
        .expect("split")
        .is_empty());
}

// ─── authentication and nicknames ──────────────────────────────────────────

#[test]
fn the_sasl_plain_payload_is_the_documented_form() {
    use base64::Engine;
    let sasl = SaslConfig {
        account: "bot".into(),
        password: "hunter2".into(),
    };
    assert!(sasl.is_configured());
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(sasl.plain_payload())
        .expect("base64");
    assert_eq!(decoded, b"\0bot\0hunter2");
    // Half-configured credentials are not credentials.
    assert!(!SaslConfig {
        account: "bot".into(),
        password: String::new()
    }
    .is_configured());
}

#[test]
fn a_long_sasl_payload_is_sent_in_protocol_sized_chunks() {
    let payload = "a".repeat(SASL_CHUNK_BYTES * 2);
    let lines = authenticate_lines(&payload);
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[0],
        format!("AUTHENTICATE {}", "a".repeat(SASL_CHUNK_BYTES))
    );
    for line in &lines {
        assert!(line.len() <= "AUTHENTICATE ".len() + SASL_CHUNK_BYTES);
    }
    // An exact multiple needs the explicit empty final chunk, or the server
    // waits forever for the rest of the payload.
    let exact = authenticate_lines(&"a".repeat(SASL_CHUNK_BYTES));
    assert_eq!(
        exact,
        vec![
            format!("AUTHENTICATE {}", "a".repeat(SASL_CHUNK_BYTES)),
            "AUTHENTICATE +".to_string()
        ]
    );
    // A short payload is one line.
    assert_eq!(authenticate_lines("short"), vec!["AUTHENTICATE short"]);
}

#[test]
fn a_colliding_nickname_gains_suffixes() {
    assert_eq!(next_nick("bot", 1), "bot_");
    assert_eq!(next_nick("bot", 3), "bot___");
    // Nicknames have to stay recognisable, so the variants are bounded and
    // every one of them differs from the next.
    assert_ne!(
        next_nick("bot", MAX_RENAMES),
        next_nick("bot", MAX_RENAMES + 1)
    );
}

#[test]
fn refusals_are_worded_so_the_delivery_queue_stops_retrying() {
    for (code, fragment) in [
        (401u16, "user not found"),
        (403, "channel not found"),
        (404, "forbidden"),
        (442, "not a member of"),
        (473, "forbidden"),
        (474, "forbidden"),
    ] {
        let reason = numeric_refusal(code, "#future", "no reason given").expect("a refusal");
        assert!(reason.contains(fragment), "{code}: {reason}");
        // The shared vocabulary must agree, or the queue retries forever.
        assert!(
            crate::delivery::is_permanent_error(&reason),
            "{code}: {reason} must read as permanent"
        );
    }
    // Numerics that are informative rather than refusals stay unclassified.
    for code in [301u16, 332, 353, 366, 421, 471, 900] {
        assert!(numeric_refusal(code, "#future", "text").is_none(), "{code}");
    }
}

// ─── configuration ─────────────────────────────────────────────────────────

#[test]
fn config_defaults_are_a_tls_client_on_the_standard_port() {
    let config: IrcConfig = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(config.port, 6697);
    assert!(config.tls);
    assert_eq!(config.idle_ping_seconds, 120);
    assert!(config.channels.is_empty());
    assert!(!config.sasl.is_configured());
}

#[test]
fn the_definition_is_usable_and_its_sample_matches_the_fields_read() {
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert!(DEFINITION.is_implemented());
    assert_eq!(DEFINITION.length_unit, LengthUnit::Bytes);
    assert!(DEFINITION.max_text_len < MAX_LINE_BYTES);
    let sample: Value = serde_json::from_str(DEFINITION.config_example).expect("valid JSON");
    // Every field the provider reads must appear in the sample, or a user
    // cannot discover it.
    let parsed: IrcConfig = serde_json::from_value(sample.clone()).expect("the sample must parse");
    for key in [
        "server",
        "port",
        "tls",
        "nick",
        "password",
        "sasl",
        "channels",
        "idle_ping_seconds",
    ] {
        assert!(sample.get(key).is_some(), "sample is missing {key}");
    }
    assert_eq!(parsed.port, 6697);
}

#[test]
fn a_missing_server_or_nick_names_the_channel() {
    for block in [
        serde_json::json!({"enabled": true, "nick": "bot"}),
        serde_json::json!({"enabled": true, "server": "irc.example"}),
    ] {
        let ctx = ctx_with(block);
        let error = match Irc::new().sender(&ctx) {
            Ok(_) => panic!("a sender must not be built without a server and nick"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("providers.irc"), "{error}");
    }
}

#[test]
fn half_configured_sasl_credentials_are_rejected_rather_than_ignored() {
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "server": "irc.example", "nick": "bot",
        "sasl": { "account": "bot", "password": "" },
    }));
    let error = match Irc::new().sender(&ctx) {
        Ok(_) => panic!("a sender must not be built with half a credential"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("sasl.account"), "{error}");
}

#[test]
fn a_port_of_zero_is_rejected() {
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "server": "irc.example", "nick": "bot", "port": 0,
    }));
    assert!(Irc::new().sender(&ctx).is_err());
}

// ─── the protocol session ──────────────────────────────────────────────────

#[tokio::test]
async fn registration_negotiates_sasl_and_joins_the_configured_channels() {
    let server = spawn_irc(
        vec![
            ":server CAP * LS :sasl multi-prefix".into(),
            ":server CAP * ACK :sasl".into(),
            "AUTHENTICATE +".into(),
            ":server 903 bot :SASL authentication successful".into(),
            ":server 001 bot :Welcome".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(
        &server,
        serde_json::json!({ "sasl": { "account": "bot", "password": "hunter2" } }),
    ));
    let task = spawn_run(&ctx);

    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await,
        "never joined: {:?}",
        server.lines()
    );
    let lines = server.lines();
    let expected = [
        "CAP LS 302",
        "NICK bot",
        "USER bot 0 * :bot",
        "CAP REQ :sasl",
        "AUTHENTICATE PLAIN",
    ];
    for (index, line) in expected.iter().enumerate() {
        assert_eq!(
            lines.get(index).map(String::as_str),
            Some(*line),
            "{lines:?}"
        );
    }
    // The payload is the base64 form of \0account\0password.
    let payload = lines
        .iter()
        .find(|line| line.starts_with("AUTHENTICATE ") && !line.contains("PLAIN"))
        .expect("a payload");
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload.trim_start_matches("AUTHENTICATE "))
        .expect("base64");
    assert_eq!(decoded, b"\0bot\0hunter2");
    // CAP END comes only after the server accepted the login.
    let cap_end = lines
        .iter()
        .position(|line| line == "CAP END")
        .expect("CAP END");
    let payload_at = lines
        .iter()
        .position(|line| line == payload)
        .expect("payload");
    assert!(cap_end > payload_at, "{lines:?}");

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_server_ping_is_answered_with_its_own_token() {
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);
    assert!(server.wait_for("JOIN", Duration::from_secs(5)).await);

    server.sent("PING :token-123");
    assert!(
        server
            .wait_for("PONG :token-123", Duration::from_secs(5))
            .await,
        "no PONG: {:?}",
        server.lines()
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn an_addressed_channel_message_reaches_the_sender_and_ctcp_does_not() {
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);
    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await
    );

    // A /me action, then a message in a channel this bridge does not answer in.
    server.sent(":alice!u@h PRIVMSG #future :\u{1}ACTION waves\u{1}");
    server.sent(":alice!u@h PRIVMSG #other :bot: hello there");
    // Only this one is a prompt. The agent is unreachable in tests, so the
    // bridge answers with its own error — which is the proof it arrived.
    server.sent(":alice!u@h PRIVMSG #future :bot: hello agent");

    assert!(
        server
            .wait_for("PRIVMSG #future", Duration::from_secs(5))
            .await,
        "no reply: {:?}",
        server.lines()
    );
    // Let anything else that was going to be sent arrive before counting.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let replies = server.lines_matching("PRIVMSG");
    assert_eq!(replies.len(), 1, "unexpected traffic: {replies:?}");
    assert!(replies[0].contains("agent"), "{replies:?}");

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_taken_nickname_is_retried_with_a_suffix() {
    let server = spawn_irc(
        vec![
            ":server 433 * bot :Nickname is already in use".into(),
            ":server 001 bot_ :Welcome".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);

    assert!(
        server.wait_for("NICK bot_", Duration::from_secs(5)).await,
        "no rename: {:?}",
        server.lines()
    );
    // Registration then completes under the new nickname.
    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await
    );
    assert!(
        server
            .wait_for("USER bot 0 * :bot", Duration::from_secs(5))
            .await
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn rejected_sasl_credentials_end_the_channel_instead_of_retrying() {
    let server = spawn_irc(
        vec![
            ":server CAP * LS :sasl".into(),
            ":server CAP * ACK :sasl".into(),
            "AUTHENTICATE +".into(),
            ":server 904 bot :SASL authentication failed".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(
        &server,
        serde_json::json!({ "sasl": { "account": "bot", "password": "wrong" } }),
    ));
    let result = tokio::time::timeout(Duration::from_secs(5), Irc::new().run(ctx)).await;
    let error = result
        .expect("run must return")
        .expect_err("bad credentials are not retryable");
    assert!(error.to_string().contains("SASL"), "{error}");
}

#[tokio::test]
async fn a_dropped_connection_is_reconnected_automatically() {
    // The listener closes each connection right after the script, so the only
    // way to see a second connection is the provider reconnecting by itself.
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], false).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);

    let reconnected = wait_until(|| server.connections() >= 2, Duration::from_secs(8)).await;
    assert!(
        reconnected,
        "the provider never reconnected (connections: {})",
        server.connections()
    );

    ctx.shutdown().notify_waiters();
    let stopped = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(stopped.is_ok(), "run must stop on shutdown");
}

#[tokio::test]
async fn an_idle_connection_is_kept_alive_with_a_client_ping() {
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(
        &server,
        serde_json::json!({ "idle_ping_seconds": 1 }),
    ));
    let task = spawn_run(&ctx);

    assert!(
        server.wait_for("PING :bot", Duration::from_secs(5)).await,
        "no keepalive: {:?}",
        server.lines()
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn run_stops_on_shutdown_without_leaving_the_socket_open() {
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);
    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await
    );

    ctx.shutdown().notify_waiters();
    let result = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(result.is_ok(), "run() must return promptly on shutdown");
    assert!(result.unwrap().unwrap().is_ok());
}

#[tokio::test]
async fn a_send_to_a_refused_target_fails_without_queueing_the_line() {
    let (out, mut rx) = mpsc::channel(OUTBOUND_QUEUE);
    let failures: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    failures.lock().insert(
        normalize("#gone"),
        numeric_refusal(442, "#gone", "nope").unwrap(),
    );
    let sender = IrcSender { out, failures };

    let error = sender
        .send_text(
            &ConversationRef {
                id: "#gone".into(),
                thread_id: None,
                kind: ChatKind::Channel,
            },
            "hello",
        )
        .await
        .expect_err("a refused target must not accept the message");
    // The reason has to read as permanent, otherwise the delivery queue would
    // retry a channel that will never accept us again.
    assert!(
        crate::delivery::is_permanent_error(&error.to_string()),
        "{error}"
    );
    assert!(rx.try_recv().is_err(), "nothing may be queued");

    // A target nobody refused is queued as one complete line.
    sender
        .send_text(
            &ConversationRef {
                id: "#future".into(),
                thread_id: None,
                kind: ChatKind::Channel,
            },
            "hello",
        )
        .await
        .unwrap();
    assert_eq!(rx.try_recv().unwrap(), "PRIVMSG #future :hello");
}

#[tokio::test]
async fn a_refusal_numeric_does_not_end_the_session() {
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);
    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await
    );

    // The bot was kicked from a channel: the session must record the refusal
    // and carry on answering everywhere else rather than dropping the link.
    server.sent(":server 442 bot #gone :You're not on that channel");
    server.sent(":alice!u@h PRIVMSG #future :bot: still there?");

    assert!(
        server
            .wait_for("PRIVMSG #future", Duration::from_secs(5))
            .await,
        "the session stopped answering: {:?}",
        server.lines()
    );
    assert!(
        !task.is_finished(),
        "a refusal numeric must not end the session"
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_full_outbound_queue_is_reported_rather_than_dropped() {
    // The connection task owns the socket, so a queue that cannot take another
    // line means the link is not keeping up — the caller must hear about it.
    let (out, _rx) = mpsc::channel(1);
    let sender = IrcSender {
        out,
        failures: Arc::new(Mutex::new(HashMap::new())),
    };
    let conversation = ConversationRef {
        id: "#future".into(),
        thread_id: None,
        kind: ChatKind::Channel,
    };
    sender.send_text(&conversation, "first").await.unwrap();
    let error = sender
        .send_text(&conversation, "second")
        .await
        .expect_err("the queue is full");
    assert!(error.to_string().contains("not keeping up"), "{error}");
}

#[tokio::test]
async fn a_send_without_a_connected_session_is_reported() {
    // `future channel send` builds a sender without connecting: the queue has
    // no receiver, and pretending the message went out would hide that.
    let (out, rx) = mpsc::channel(OUTBOUND_QUEUE);
    drop(rx);
    let sender = IrcSender {
        out,
        failures: Arc::new(Mutex::new(HashMap::new())),
    };
    let error = sender
        .send_text(
            &ConversationRef {
                id: "#future".into(),
                thread_id: None,
                kind: ChatKind::Channel,
            },
            "hello",
        )
        .await
        .expect_err("nothing is connected");
    let message = error.to_string();
    assert!(message.contains("not connected"), "{message}");
    // Worth retrying once the bridge is up, so it must not read as permanent.
    assert!(!crate::delivery::is_permanent_error(&message), "{message}");
}

#[tokio::test]
async fn a_sender_reports_the_irc_declaration_without_connecting() {
    let server = spawn_irc(vec![], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let sender = Irc::new()
        .sender(&ctx)
        .expect("a valid configuration needs no server");
    assert_eq!(sender.definition().id, "irc");
    assert_eq!(sender.definition().length_unit, LengthUnit::Bytes);
    // Building a sender must not have opened anything: `future channel send`
    // on a machine with no bridge running must still construct one.
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn an_over_long_line_from_the_server_ends_the_connection() {
    // 16 KiB with no line break is not IRC; the reader must give up rather
    // than growing its buffer until the process runs out of memory. The bytes
    // are written raw, because the mock normally terminates every line.
    let payload = vec![b'x'; MAX_INBOUND_LINE_BYTES + 16];
    let server = spawn_irc_raw(&payload, true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);

    let reconnected = wait_until(|| server.connections() >= 2, Duration::from_secs(10)).await;
    assert!(
        reconnected,
        "the nonsense line must be treated as a dropped connection (connections: {})",
        server.connections()
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_refused_capability_negotiation_does_not_stop_registration() {
    let server = spawn_irc(
        vec![
            ":server CAP * LS :sasl multi-prefix".into(),
            ":server CAP * ACK :sasl".into(),
            // A challenge that is not the `+` greeting: the payload must not be
            // sent on the back of it.
            "AUTHENTICATE *".into(),
            ":server CAP * NAK :sasl".into(),
            ":server 001 bot :Welcome".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(
        &server,
        serde_json::json!({ "sasl": { "account": "bot", "password": "hunter2" } }),
    ));
    let task = spawn_run(&ctx);

    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await,
        "registration must survive a refused capability: {:?}",
        server.lines()
    );
    let lines = server.lines();
    // The server refused SASL, so registration proceeds without it.
    assert!(lines.iter().any(|line| line == "CAP END"), "{lines:?}");
    // Only the `AUTHENTICATE PLAIN` request went out; no credential followed it.
    let payloads = lines
        .iter()
        .filter(|line| line.starts_with("AUTHENTICATE ") && !line.contains("PLAIN"))
        .count();
    assert_eq!(payloads, 0, "the payload must wait for `+`: {lines:?}");

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_join_echo_clears_an_earlier_refusal() {
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);
    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await
    );

    // Kicked: the server says the bot is not on the channel, so a reply is
    // refused instead of being queued for a target that cannot take it.
    server.sent(":server 442 bot #future :You're not on that channel");
    server.sent(":alice!u@h PRIVMSG #future :bot: are you there?");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        server.lines_matching("PRIVMSG #future").is_empty(),
        "a refused target must not be written to: {:?}",
        server.lines()
    );

    // Back in the channel: being re-joined is what makes it writable again.
    server.sent(":bot!u@h JOIN #future");
    server.sent(":alice!u@h PRIVMSG #future :bot: hello again");
    assert!(
        server
            .wait_for("PRIVMSG #future", Duration::from_secs(5))
            .await,
        "the join echo must clear the refusal: {:?}",
        server.lines()
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_server_error_closes_the_connection_and_it_reconnects() {
    let server = spawn_irc(
        vec![
            ":server 001 bot :Welcome".into(),
            "ERROR :Closing Link: bot (Quit)".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);

    // An `ERROR` line is the server hanging up on purpose; the channel comes
    // back rather than dying.
    let reconnected = wait_until(|| server.connections() >= 2, Duration::from_secs(8)).await;
    assert!(reconnected, "the channel never came back");

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn an_unusable_nickname_is_fatal() {
    let server = spawn_irc(vec![":server 432 * :Erroneous Nickname".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let result = tokio::time::timeout(Duration::from_secs(5), Irc::new().run(ctx)).await;
    let error = result
        .expect("run must return")
        .expect_err("renaming cannot fix a rejected nickname");
    assert!(error.to_string().contains("rejects"), "{error}");
}

#[tokio::test]
async fn a_banned_client_is_fatal() {
    let server = spawn_irc(vec![":server 465 * :You are banned".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let result = tokio::time::timeout(Duration::from_secs(5), Irc::new().run(ctx)).await;
    let error = result
        .expect("run must return")
        .expect_err("a ban is not worth retrying");
    assert!(error.to_string().contains("banned"), "{error}");
}

#[tokio::test]
async fn a_nickname_that_never_frees_ends_the_channel() {
    // Every variant is taken too: renaming forever would hammer the network.
    let collisions: Vec<String> = (0..MAX_RENAMES + 2)
        .map(|_| ":server 433 * bot :Nickname is already in use".to_string())
        .collect();
    let server = spawn_irc(collisions, true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let result = tokio::time::timeout(Duration::from_secs(5), Irc::new().run(ctx)).await;
    let error = result
        .expect("run must return")
        .expect_err("a nickname that never frees is fatal");
    assert!(error.to_string().contains("taken"), "{error}");
    // It tried the bounded number of variants and no more.
    let renames = server.lines_matching("NICK bot_").len();
    assert!(renames <= MAX_RENAMES as usize, "{renames} renames");
}

#[tokio::test]
async fn a_login_numeric_and_an_unknown_numeric_are_only_noted() {
    let server = spawn_irc(
        vec![
            // A blank line is not a message and must not end the connection.
            "".into(),
            ":server 900 bot bot!u@h :You are now logged in as bot".into(),
            ":server 999 bot :Something nobody has heard of".into(),
            ":server 001 bot :Welcome".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);

    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await,
        "informational numerics must not stop registration: {:?}",
        server.lines()
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

// ─── TLS ───────────────────────────────────────────────────────────────────

/// A self-signed certificate for these tests only, generated once and embedded
/// as data: a test that never leaves the loopback interface must not add a
/// certificate-generation dependency to the crate. Valid to 2035, SAN
/// `IP:127.0.0.1`.
const TEST_CERT_DER_B64: &str = "MIIBrTCCAVOgAwIBAgIUPcdm1/isjsejaBpxUxr9OSyZCiowCgYIKoZIzj0EAwIwHjEcMBoGA1UEAwwTZnV0dXJlLWNoYW5uZWwtdGVzdDAeFw0yNjA5MjIwNjAwMTNaFw0zNjA5MTkwNjAwMTNaMB4xHDAaBgNVBAMME2Z1dHVyZS1jaGFubmVsLXRlc3QwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAS9qplZNsvpiONd4vcb6GL8/1aJ6FBaUGd+RRyy1O8zv2ddVdtlAy2Cr5gDvtWRakjaYpP8j8irGg0lCretgeyGo28wbTAdBgNVHQ4EFgQUcyZxwrgynNukfop1QCI53yYJAq4wHwYDVR0jBBgwFoAUcyZxwrgynNukfop1QCI53yYJAq4wDwYDVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARhwR/AAABgglsb2NhbGhvc3QwCgYIKoZIzj0EAwIDSAAwRQIgCdxFxfhv1RBSquhGXM+a7Y7sjvJPaf17hg4I9nqj2FsCIQCYHdYjqGWipWsesfZFSnGWj7cg8rG+LG5TdVhWwhQN6Q==";

/// The PKCS#8 private key for [`TEST_CERT_DER_B64`].
const TEST_KEY_DER_B64: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgdThDQAVQuOklcIFkvRg+vjevKPiUe7qPc6zo5frpCy+hRANCAAS9qplZNsvpiONd4vcb6GL8/1aJ6FBaUGd+RRyy1O8zv2ddVdtlAy2Cr5gDvtWRakjaYpP8j8irGg0lCretgeyG";

fn decode_b64(encoded: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("embedded test data must be valid base64")
}

/// A verifier that accepts the certificate above.
///
/// The production client verifies against the platform trust store, which
/// cannot know a certificate invented for a test; everything else about the
/// handshake (signatures, record layer, key schedule) is still checked.
#[derive(Debug)]
struct TrustTestCertificate;

impl rustls::client::danger::ServerCertVerifier for TrustTestCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &test_algorithms())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &test_algorithms())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        test_algorithms().supported_schemes()
    }
}

fn test_algorithms() -> rustls::crypto::WebPkiSupportedAlgorithms {
    rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms
}

/// The client TLS configuration these tests use instead of the platform store.
fn trusting_tls() -> rustls::ClientConfig {
    crate::test_support::ensure_crypto_provider();
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TrustTestCertificate))
        .with_no_client_auth()
}

/// How a TLS test server ends a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Close {
    /// Drop the socket during the handshake, so it can never complete.
    DuringHandshake,
    /// A clean TLS closure: `close_notify`, then the socket goes away.
    Clean,
    /// The socket disappears with no `close_notify` — a truncated connection.
    Unclean,
    /// Serve until the test asks for one of the above.
    Await,
}

/// A TLS listener that speaks IRC, with the same recording surface as the
/// plaintext mock.
async fn spawn_tls_irc(script: Vec<String>, initial: Close) -> MockIrc {
    crate::test_support::ensure_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (out, out_rx) = mpsc::unbounded_channel::<String>();
    let out_rx = Arc::new(tokio::sync::Mutex::new(out_rx));
    let (closer, close_rx) = mpsc::unbounded_channel::<Close>();
    let close_rx = Arc::new(tokio::sync::Mutex::new(close_rx));
    let config = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(decode_b64(
                    TEST_CERT_DER_B64,
                ))],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(decode_b64(TEST_KEY_DER_B64)),
                ),
            )
            .expect("server TLS configuration"),
    );
    let received_task = received.clone();
    let connections_task = connections.clone();

    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            connections_task.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let config = config.clone();
            let script = script.clone();
            let received = received_task.clone();
            let out_rx = out_rx.clone();
            let close_rx = close_rx.clone();
            tokio::spawn(async move {
                if initial == Close::DuringHandshake {
                    // Read the ClientHello first and then walk away: a socket
                    // closed with unread data in it answers with a reset, and
                    // the client has to see an orderly end of stream instead.
                    let mut hello = [0u8; 4096];
                    let _ = tokio::io::AsyncReadExt::read(&mut tcp, &mut hello).await;
                    drop(tcp);
                    return;
                }
                let mut conn = rustls::ServerConnection::new(config).expect("server connection");
                let (mut read, mut write) = tokio::io::split(tcp);

                // Handshake: feed the client's records in, push ours out.
                while conn.is_handshaking() {
                    let mut raw = [0u8; 8192];
                    match read.read(&mut raw).await {
                        Ok(0) | Err(_) => return,
                        Ok(read_bytes) => {
                            if feed(&mut conn, &raw[..read_bytes]).is_err() {
                                return;
                            }
                            if flush(&mut conn, &mut write).await.is_err() {
                                return;
                            }
                        }
                    }
                }

                for line in &script {
                    if send_line(&mut conn, &mut write, line).await.is_err() {
                        return;
                    }
                }

                // One guard for the whole connection: a lock taken inside a
                // `select!` arm does not outlive the arm's body.
                let mut close_rx = close_rx.lock().await;
                let mut out_rx = out_rx.lock().await;
                let mut pending: Vec<u8> = Vec::new();
                loop {
                    let mut raw = [0u8; 8192];
                    let event = tokio::select! {
                        result = read.read(&mut raw) => TlsEvent::Read(result),
                        Some(line) = out_rx.recv() => TlsEvent::Send(line),
                        Some(how) = close_rx.recv() => TlsEvent::Close(how),
                    };
                    match event {
                        // A polite closure tells the client the stream ended;
                        // a truncated one just drops the socket, which is the
                        // difference the client has to survive.
                        TlsEvent::Close(Close::Clean) => {
                            conn.send_close_notify();
                            let _ = flush(&mut conn, &mut write).await;
                            return;
                        }
                        TlsEvent::Close(Close::Unclean)
                        | TlsEvent::Close(Close::DuringHandshake) => {
                            return;
                        }
                        TlsEvent::Close(Close::Await) => {}
                        TlsEvent::Send(line) => {
                            if send_line(&mut conn, &mut write, &line).await.is_err() {
                                return;
                            }
                        }
                        TlsEvent::Read(Ok(0)) | TlsEvent::Read(Err(_)) => return,
                        TlsEvent::Read(Ok(read_bytes)) => {
                            if feed(&mut conn, &raw[..read_bytes]).is_err() {
                                return;
                            }
                            if flush(&mut conn, &mut write).await.is_err() {
                                return;
                            }
                            pending.extend_from_slice(&take_plaintext(&mut conn));
                            while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                                let line: Vec<u8> = pending.drain(..=index).collect();
                                let line = &line[..line.len() - 1];
                                let line = line.strip_suffix(b"\r").unwrap_or(line);
                                received
                                    .lock()
                                    .push(String::from_utf8_lossy(line).into_owned());
                            }
                        }
                    }
                }
            });
        }
    });

    MockIrc {
        addr,
        received,
        out,
        connections,
        closer,
    }
}

/// One action the TLS test server takes.
enum TlsEvent {
    Read(std::io::Result<usize>),
    Send(String),
    Close(Close),
}

/// Hand bytes to rustls.
fn feed(conn: &mut rustls::ServerConnection, bytes: &[u8]) -> std::io::Result<()> {
    let mut cursor = std::io::Cursor::new(bytes);
    while (cursor.position() as usize) < bytes.len() {
        conn.read_tls(&mut cursor)?;
        conn.process_new_packets()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    }
    Ok(())
}

/// Push rustls's pending records to the socket.
async fn flush<W: tokio::io::AsyncWriteExt + Unpin>(
    conn: &mut rustls::ServerConnection,
    write: &mut W,
) -> std::io::Result<()> {
    while conn.wants_write() {
        let mut out = Vec::new();
        conn.write_tls(&mut out)?;
        write.write_all(&out).await?;
    }
    write.flush().await
}

/// Write one line over the TLS connection and push it out.
async fn send_line<W: tokio::io::AsyncWriteExt + Unpin>(
    conn: &mut rustls::ServerConnection,
    write: &mut W,
    line: &str,
) -> std::io::Result<()> {
    let bytes = format!("{line}\r\n");
    std::io::Write::write_all(&mut conn.writer(), bytes.as_bytes())?;
    flush(conn, write).await
}

/// Everything the client has written that rustls has decrypted so far.
fn take_plaintext(conn: &mut rustls::ServerConnection) -> Vec<u8> {
    let mut decrypted = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match std::io::Read::read(&mut conn.reader(), &mut buf) {
            Ok(0) | Err(_) => return decrypted,
            Ok(read) => decrypted.extend_from_slice(&buf[..read]),
        }
    }
}

/// The provider as the TLS tests use it: their own trust anchor, since the
/// platform store cannot know a certificate generated for a test.
fn trusting_provider() -> Irc {
    Irc {
        tls: Some(Arc::new(trusting_tls())),
    }
}

/// Run the trusting provider in the background.
fn spawn_trusting_run(provider: Irc, ctx: &ProviderCtx) -> tokio::task::JoinHandle<Result<()>> {
    let ctx = ctx.clone();
    tokio::spawn(async move { provider.run(ctx).await })
}

/// A TLS configuration pointing at a loopback server.
fn tls_config_for(server: &MockIrc, extra: Value) -> Value {
    let mut block = config_for(server, extra);
    if let Some(object) = block.as_object_mut() {
        object.insert("tls".into(), Value::Bool(true));
    }
    block
}

#[tokio::test]
async fn a_tls_probe_handshakes_and_reports_the_transport() {
    let server = spawn_tls_irc(vec![":server 001 bot :Welcome".into()], Close::Await).await;
    let ctx = ctx_with(tls_config_for(&server, serde_json::json!({})));
    let summary = trusting_provider()
        .probe(&ctx)
        .await
        .expect("the handshake must complete");
    assert!(summary.contains("TLS"), "{summary}");
    assert!(summary.contains("bot"), "{summary}");
    // The registration burst travelled over the encrypted connection.
    assert!(server.wait_for("NICK bot", Duration::from_secs(5)).await);
    server.close(Close::Clean);
}

#[tokio::test]
async fn a_tls_session_registers_replies_and_reconnects_after_a_clean_close() {
    let server = spawn_tls_irc(vec![":server 001 bot :Welcome".into()], Close::Await).await;
    let ctx = ctx_with(tls_config_for(&server, serde_json::json!({})));
    let task = spawn_trusting_run(trusting_provider(), &ctx);

    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await,
        "registration over TLS: {:?}",
        server.lines()
    );
    // A message decrypts, reaches the bridge, and the reply is encrypted back.
    server.sent(":alice!u@h PRIVMSG #future :bot: over tls");
    assert!(
        server
            .wait_for("PRIVMSG #future", Duration::from_secs(5))
            .await,
        "a reply must travel over TLS: {:?}",
        server.lines()
    );

    // The server closes the session politely: the client comes back.
    server.close(Close::Clean);
    let reconnected = wait_until(|| server.connections() >= 2, Duration::from_secs(8)).await;
    assert!(reconnected, "a clean close must not end the channel");

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_truncated_tls_handshake_is_reported() {
    let server = spawn_tls_irc(vec![], Close::DuringHandshake).await;
    let ctx = ctx_with(tls_config_for(&server, serde_json::json!({})));
    let error = Irc::new()
        .probe(&ctx)
        .await
        .expect_err("a socket that answers nothing is not a working channel");
    assert!(error.to_string().contains("TLS handshake"), "{error}");
}

#[tokio::test]
async fn a_truncated_tls_connection_is_treated_as_a_dropped_one() {
    // No `close_notify`: the client has to notice the socket died rather than
    // wait for a response that will never come.
    let server = spawn_tls_irc(vec![":server 001 bot :Welcome".into()], Close::Await).await;
    let ctx = ctx_with(tls_config_for(&server, serde_json::json!({})));
    let task = spawn_trusting_run(trusting_provider(), &ctx);
    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await
    );

    server.close(Close::Unclean);
    let reconnected = wait_until(|| server.connections() >= 2, Duration::from_secs(8)).await;
    assert!(reconnected, "a truncated connection must be retried");

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_capability_answer_we_did_not_ask_for_and_one_nobody_knows() {
    let server = spawn_irc(
        vec![
            ":server CAP * LS :sasl".into(),
            // An acknowledgement for a capability this client only announced
            // interest in: there is nothing to negotiate, so registration
            // continues without SASL.
            ":server CAP * ACK :multi-prefix".into(),
            ":server 001 bot :Welcome".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(
        &server,
        serde_json::json!({ "sasl": { "account": "bot", "password": "hunter2" } }),
    ));
    let task = spawn_run(&ctx);

    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await,
        "an unrelated acknowledgement must not block registration: {:?}",
        server.lines()
    );
    let lines = server.lines();
    assert!(lines.iter().any(|line| line == "CAP END"), "{lines:?}");
    // No credential followed, because the capability was never agreed to.
    assert!(
        !lines.iter().any(|line| line.contains("PLAIN")),
        "{lines:?}"
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn an_unknown_capability_subcommand_is_ignored() {
    let server = spawn_irc(
        vec![
            ":server CAP * LS :sasl".into(),
            // A subcommand the protocol does not define: not worth acting on.
            ":server CAP * WHATEVER :sasl".into(),
            ":server 001 bot :Welcome".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(
        &server,
        serde_json::json!({ "sasl": { "account": "bot", "password": "hunter2" } }),
    ));
    let task = spawn_run(&ctx);

    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await,
        "an unknown capability subcommand must not stop registration: {:?}",
        server.lines()
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

#[tokio::test]
async fn a_join_without_a_prefix_is_harmless() {
    let server = spawn_irc(vec![":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let task = spawn_run(&ctx);
    assert!(
        server
            .wait_for("JOIN #future", Duration::from_secs(5))
            .await
    );

    // Servers in the wild have been known to omit the prefix; there is then no
    // nickname to compare, and the line is simply not ours to act on.
    server.sent("JOIN #somewhere");
    server.sent(":alice!u@h PRIVMSG #future :bot: still here?");
    assert!(
        server
            .wait_for("PRIVMSG #future", Duration::from_secs(5))
            .await,
        "a prefix-less line must not stop the session: {:?}",
        server.lines()
    );

    ctx.shutdown().notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}

// ─── probing ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_probe_reports_the_transport_and_the_nickname() {
    let server = spawn_irc(
        vec![
            ":server 001 bot :Welcome".into(),
            ":server 376 bot :End of MOTD".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let summary = Irc::new()
        .probe(&ctx)
        .await
        .expect("a welcome proves the login");
    assert!(summary.contains("connected"), "{summary}");
    assert!(summary.contains("TCP"), "{summary}");
    assert!(summary.contains("bot"), "{summary}");
    assert!(server.wait_for("NICK bot", Duration::from_secs(5)).await);
}

#[tokio::test]
async fn a_probe_skips_a_line_it_cannot_read() {
    let server = spawn_irc(vec!["".into(), ":server 001 bot :Welcome".into()], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let summary = Irc::new()
        .probe(&ctx)
        .await
        .expect("a welcome follows the blank line");
    assert!(summary.contains("bot"), "{summary}");
}

#[tokio::test]
async fn a_probe_that_never_registers_gives_up() {
    // A server that accepts the connection and then says nothing: the probe has
    // to stop waiting rather than hold `future channel test` forever. The
    // budget is shortened so the give-up path is exercised without the wait.
    let server = spawn_irc(vec![], true).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let error = Irc::new()
        .probe_with_deadline(&ctx, Duration::from_millis(200))
        .await
        .expect_err("a silent server is not a working channel");
    let message = error.to_string();
    assert!(
        message.contains("never completed IRC registration"),
        "{message}"
    );
    assert!(server.wait_for("NICK bot", Duration::from_secs(5)).await);
}

#[tokio::test]
async fn a_probe_that_loses_the_socket_before_the_welcome_gives_up() {
    // The server accepts and then goes away: there is no registration to wait
    // for, so the probe must fail rather than sit until its budget runs out.
    let server = spawn_irc_raw(&[], false).await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let error = Irc::new()
        .probe_with_deadline(&ctx, Duration::from_secs(5))
        .await
        .expect_err("a closed socket is not a working channel");
    assert!(
        error
            .to_string()
            .contains("never completed IRC registration"),
        "{error}"
    );
}

#[tokio::test]
async fn a_probe_keeps_reading_past_a_line_that_is_not_the_welcome() {
    // A server may say other things first (a login notice, a MOTD line): the
    // probe has to carry on rather than treat the first line as the answer.
    let server = spawn_irc(
        vec![
            ":server 900 bot bot!u@h :You are now logged in as bot".into(),
            ":server 376 bot :End of MOTD".into(),
            ":server 001 bot :Welcome".into(),
        ],
        true,
    )
    .await;
    let ctx = ctx_with(config_for(&server, serde_json::json!({})));
    let summary = Irc::new()
        .probe(&ctx)
        .await
        .expect("the welcome arrives after the notices");
    assert!(summary.contains("connected"), "{summary}");
    assert!(summary.contains("bot"), "{summary}");
}

#[tokio::test]
async fn a_probe_never_claims_success_without_a_server() {
    let ctx = ctx_with(serde_json::json!({
        "enabled": true, "server": "127.0.0.1", "port": 1, "tls": false, "nick": "bot",
    }));
    let error = Irc::new().probe(&ctx).await.expect_err("must fail");
    assert!(error.to_string().contains("connect"), "{error}");
}

#[tokio::test]
async fn a_probe_reports_a_server_that_rejects_the_credentials() {
    let server = spawn_irc(vec![":server 464 * :Password incorrect".into()], true).await;
    let ctx = ctx_with(config_for(
        &server,
        serde_json::json!({ "password": "wrong" }),
    ));
    let error = Irc::new().probe(&ctx).await.expect_err("must fail");
    assert!(error.to_string().contains("password"), "{error}");
    // `PASS` must precede `NICK`, or the server ignores it. The probe returns as
    // soon as the rejection arrives, so wait for the client's whole registration
    // burst to be on the wire before comparing positions.
    assert!(
        server.wait_for("NICK bot", Duration::from_secs(5)).await,
        "{:?}",
        server.lines()
    );
    let lines = server.lines();
    let pass = lines.iter().position(|line| line.starts_with("PASS "));
    let nick = lines.iter().position(|line| line.starts_with("NICK "));
    assert!(pass.is_some() && nick.is_some(), "{lines:?}");
    assert!(pass.unwrap() < nick.unwrap(), "{lines:?}");
}
