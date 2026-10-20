//! `future relay` — the self-hosted relay's command line.
//!
//! Two verbs, because a relay is two things: a trust setup an operator creates
//! once (`init`), and a process that turns signed invitations into short-lived
//! NATS credentials (`serve`). It runs inside the `future` binary rather than as
//! its own executable: same code, one fewer artifact to install, and an
//! operator already has `future` wherever the desktop is.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context};
use nkeys::KeyPair;

use crate::relay::config::RelayConfig;
use crate::relay::jwt;
use crate::relay::server::{now, router, Relay};

pub const HELP: &str = "future relay — an account-free NATS relay for FutureOS remote control

Usage:
  future relay init  --dir DIR [--listen ADDR] [--public-url URL]
                     [--nats-url URL] [--nats-ws-url URL]
                     [--tls-cert PEM --tls-key PEM] [--force]
  future relay serve --config FILE
  future relay --help

  init   Create the relay's trust setup: an operator and account NKey, their
         JWTs, a nats-server configuration, and relay.json. Nothing is
         overwritten unless --force is given.
  serve  Run the HTTP API that turns a signed invitation into a short-lived
         NATS credential.

Defaults for init:
  --listen        127.0.0.1:8099
  --public-url    http://127.0.0.1:8099
  --nats-url      nats://127.0.0.1:4222
  --nats-ws-url   ws://127.0.0.1:9090

The relay has no accounts and no database. It keeps rooms in a JSON file beside
relay.json, so a restart does not cost a re-pair, and it never stores a
credential: only hashes of the invitation and refresh tokens.

A desktop points at it with `futureos --relay URL` or
`future headless --relay URL`; the invitation it prints then carries the
relay's address, and the phone talks to the relay instead of the Future
platform. Devices paired through the Future platform keep working: the choice
is recorded per pairing.";

pub fn run(args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h" | "help") {
        println!("{HELP}");
        return Ok(());
    }
    match args[0].as_str() {
        "init" => init(&args[1..]),
        "serve" => serve(&args[1..]),
        other => bail!("unknown command `{other}`; run `future relay --help`"),
    }
}

struct InitArgs {
    dir: PathBuf,
    listen: String,
    public_url: String,
    nats_url: String,
    nats_ws_url: String,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    force: bool,
}

fn parse_init(args: &[String]) -> anyhow::Result<InitArgs> {
    let mut parsed = InitArgs {
        dir: PathBuf::from("."),
        listen: "127.0.0.1:8099".into(),
        public_url: "http://127.0.0.1:8099".into(),
        nats_url: "nats://127.0.0.1:4222".into(),
        nats_ws_url: "ws://127.0.0.1:9090".into(),
        tls_cert: None,
        tls_key: None,
        force: false,
    };
    let mut index = 0;
    let mut dir_seen = false;
    while index < args.len() {
        let flag = args[index].clone();
        let mut value = || -> anyhow::Result<String> {
            index += 1;
            args.get(index)
                .cloned()
                .with_context(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--dir" => {
                parsed.dir = PathBuf::from(value()?);
                dir_seen = true;
            }
            "--listen" => parsed.listen = value()?,
            "--public-url" => parsed.public_url = value()?,
            "--nats-url" => parsed.nats_url = value()?,
            "--nats-ws-url" => parsed.nats_ws_url = value()?,
            "--tls-cert" => parsed.tls_cert = Some(PathBuf::from(value()?)),
            "--tls-key" => parsed.tls_key = Some(PathBuf::from(value()?)),
            "--force" => parsed.force = true,
            other => bail!("unknown option `{other}`"),
        }
        index += 1;
    }
    if !dir_seen {
        bail!("init needs --dir DIR (the directory the relay's files live in)");
    }
    if parsed.tls_cert.is_some() != parsed.tls_key.is_some() {
        bail!("--tls-cert and --tls-key go together");
    }
    Ok(parsed)
}

fn init(args: &[String]) -> anyhow::Result<()> {
    let args = parse_init(args)?;
    std::fs::create_dir_all(&args.dir).with_context(|| format!("create {}", args.dir.display()))?;

    let paths = TrustFiles::in_dir(&args.dir);
    if !args.force {
        for path in paths.all() {
            if path.exists() {
                bail!(
                    "{} already exists; pass --force to replace the trust setup",
                    path.display()
                );
            }
        }
    }

    let operator = KeyPair::new_operator();
    let account = KeyPair::new_account();
    let iat = now();

    let operator_jwt =
        jwt::operator_jwt(&operator, "FUTURE-RELAY", iat).map_err(anyhow::Error::msg)?;
    let account_jwt =
        jwt::account_jwt(&operator, &account, "RELAY", iat).map_err(anyhow::Error::msg)?;

    write_secret(&paths.operator_seed, &operator.seed()?)?;
    write_secret(&paths.account_seed, &account.seed()?)?;
    write_file(&paths.operator_jwt, &format!("{operator_jwt}\n"))?;
    write_file(&paths.account_jwt, &format!("{account_jwt}\n"))?;
    write_file(
        &paths.nats_conf,
        &nats_conf(
            &paths.operator_jwt,
            &account.public_key(),
            &account_jwt,
            &args.nats_url,
            &args.nats_ws_url,
            args.tls_cert.as_deref().zip(args.tls_key.as_deref()),
        ),
    )?;

    let config = RelayConfig {
        listen: args.listen.clone(),
        public_url: args.public_url.trim_end_matches('/').to_string(),
        nats_url: args.nats_url.clone(),
        nats_ws_url: args.nats_ws_url.clone(),
        account_seed: account.seed()?,
        jwt_ttl_seconds: 300,
        invitation_ttl_seconds: 300,
        room_ttl_seconds: 90 * 24 * 60 * 60,
        subs_limit: -1,
        payload_limit: jwt::DEFAULT_PAYLOAD_LIMIT,
        http_timeout_seconds: 15,
    };
    config.save(&paths.config)?;

    println!("Relay trust setup written to {}", args.dir.display());
    println!("  operator public key : {}", operator.public_key());
    println!("  account  public key : {}", account.public_key());
    println!();
    println!("Next:");
    println!("  1. nats-server -c {}", paths.nats_conf.display());
    println!(
        "  2. future relay serve --config {}",
        paths.config.display()
    );
    println!();
    println!("Keep operator.seed and account.seed private: the account seed signs every");
    println!("NATS credential this relay issues. Never commit or share them.");
    Ok(())
}

struct TrustFiles {
    operator_seed: PathBuf,
    account_seed: PathBuf,
    operator_jwt: PathBuf,
    account_jwt: PathBuf,
    nats_conf: PathBuf,
    config: PathBuf,
}

impl TrustFiles {
    fn in_dir(dir: &Path) -> Self {
        Self {
            operator_seed: dir.join("operator.seed"),
            account_seed: dir.join("account.seed"),
            operator_jwt: dir.join("operator.jwt"),
            account_jwt: dir.join("account.jwt"),
            nats_conf: dir.join("nats.conf"),
            config: dir.join("relay.json"),
        }
    }

    fn all(&self) -> Vec<&Path> {
        vec![
            &self.operator_seed,
            &self.account_seed,
            &self.operator_jwt,
            &self.account_jwt,
            &self.nats_conf,
            &self.config,
        ]
    }
}

fn nats_conf(
    operator_jwt: &Path,
    account_public_key: &str,
    account_jwt: &str,
    nats_url: &str,
    nats_ws_url: &str,
    tls: Option<(&Path, &Path)>,
) -> String {
    // The advertised endpoints and the listeners have to agree, or the relay
    // hands out a credential for a port nothing listens on. Only the port is
    // taken from the URL: the host there is the public name, which is not
    // necessarily an address this machine can bind.
    let client_port = port_of(nats_url, 4222);
    let ws_port = port_of(nats_ws_url, 9090);
    // Deliberately no JetStream: remote control is NATS Core only, and a store
    // would be a place for ciphertext and metadata to accumulate on a relay
    // that is not supposed to keep either.
    let operator = absolute(operator_jwt);
    // TLS protects the credential exchange and the subject metadata. It is not
    // a substitute for the Noise layer on top, and the shipping clients refuse
    // to connect without it (the phone requires `wss://`, the desktop verifies
    // the peer), so a deployment that skips it simply does not work.
    let tls_block = match tls {
        Some((cert, key)) => format!(
            "\ntls {{\n  cert_file: \"{}\"\n  key_file: \"{}\"\n}}\n",
            absolute(cert),
            absolute(key)
        ),
        None => String::new(),
    };
    let ws_tls = if tls.is_some() {
        "  tls {}\n".to_string()
    } else {
        "  # Terminate TLS here (cert_file/key_file) for a public deployment: the\n\
         \x20 # shipping phone client refuses anything but wss://.\n\
         \x20 no_tls: true\n"
            .to_string()
    };
    format!(
        "# Generated by `future relay init`.\n\
         #\n\
         # One operator, one account. Every connection authenticates with a\n\
         # short-lived user JWT scoped to one pairing's subjects; `future relay\n\
         # serve` is what mints them. JetStream is off on purpose.\n\
         server_name: future-relay\n\
         \n\
         listen: 0.0.0.0:{client_port}\n\
         {tls_block}\n\
         # Bind the monitoring endpoint to loopback only, or remove this line.\n\
         http: 127.0.0.1:8222\n\
         \n\
         operator: \"{operator}\"\n\
         # The account JWTs live in this file (resolver_preload), so the resolver\n\
         # only has to accept preloads: no directory, no JetStream store.\n\
         resolver: MEMORY\n\
         \n\
         resolver_preload: {{\n\
         \x20 {account_public_key}: \"{account_jwt}\"\n\
         }}\n\
         \n\
         websocket {{\n\
         \x20 host: 0.0.0.0\n\
         \x20 port: {ws_port}\n\
         {ws_tls}\
         }}\n"
    )
}

/// The port of a `scheme://host:port[/...]` URL, or `default`.
fn port_of(url: &str, default: u16) -> u16 {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or_default();
    authority
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .unwrap_or(default)
}

fn absolute(path: &Path) -> String {
    // `canonicalize` returns the Windows verbatim form (`\\?\D:\…`), which is not
    // a path nats-server would open. Strip it: the generated file is meant to be
    // readable and copy-pasteable by a person.
    let text = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let text = match text.strip_prefix("\\\\?\\") {
        Some(stripped) => stripped.to_string(),
        None => text,
    };
    text.replace('\\', "/")
}

fn write_secret(path: &Path, contents: &str) -> anyhow::Result<()> {
    write_file(path, &format!("{contents}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn write_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

/// `serve` blocks until the process is stopped. Its runtime is built here and
/// the CLI dispatches it before entering its own, the same rule the embedded
/// `agent`/`tui`/`channel`/`goal` groups follow.
pub fn serve(args: &[String]) -> anyhow::Result<()> {
    let mut config: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--config" => {
                index += 1;
                config = Some(PathBuf::from(
                    args.get(index).context("--config needs a path")?,
                ));
            }
            other => bail!("unknown option `{other}`"),
        }
        index += 1;
    }
    let config_path =
        config.context("serve needs --config FILE (written by `future relay init`)")?;
    let config = RelayConfig::load(&config_path)?;
    let listen: SocketAddr = config
        .listen
        .parse()
        .with_context(|| format!("listen address `{}`", config.listen))?;

    let rooms_path = config_path.with_file_name("rooms.json");
    let relay = Arc::new(Relay::new(config, Some(&rooms_path))?);
    println!(
        "future relay listening on http://{listen} (account {}, public {}, rooms {})",
        relay.account_public_key(),
        relay.config.public_url,
        relay.rooms.len()
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build the tokio runtime")?;
    runtime.block_on(async move {
        let app = router(relay.clone());
        let rooms = relay.rooms_probe();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                let dropped = rooms.purge(now());
                if dropped > 0 {
                    eprintln!("future relay: pruned {dropped} expired room(s)");
                }
            }
        });
        let listener = tokio::net::TcpListener::bind(listen).await?;
        axum::serve(listener, app).await?;
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_is_read_from_the_advertised_url_or_defaulted() {
        assert_eq!(port_of("nats://relay.example:4223", 4222), 4223);
        assert_eq!(port_of("wss://relay.example/ws", 9090), 9090);
        assert_eq!(port_of("nats://relay.example", 4222), 4222);
        assert_eq!(port_of("nats://[::1]:14222", 4222), 14222);
    }

    #[test]
    fn unknown_verbs_and_flags_are_refused() {
        assert!(run(&["bogus".to_string()]).is_err());
        assert!(run(&["init".to_string()]).is_err(), "init needs --dir");
        assert!(run(&["serve".to_string()]).is_err(), "serve needs --config");
        assert!(run(&["serve".to_string(), "--bogus".to_string()]).is_err());
    }

    #[test]
    fn init_writes_a_trust_setup_and_refuses_to_clobber_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        run(&["init".to_string(), "--dir".to_string(), path.clone()]).unwrap();
        for name in [
            "operator.seed",
            "account.seed",
            "operator.jwt",
            "account.jwt",
            "nats.conf",
            "relay.json",
        ] {
            assert!(dir.path().join(name).exists(), "{name}");
        }
        let config = RelayConfig::load(&dir.path().join("relay.json")).unwrap();
        config.validate().unwrap();
        // A second init would replace the account key every credential was
        // signed with, so it is refused unless the operator says so.
        assert!(run(&["init".to_string(), "--dir".to_string(), path.clone()]).is_err());
        run(&[
            "init".to_string(),
            "--dir".to_string(),
            path,
            "--force".to_string(),
        ])
        .unwrap();
    }

    #[test]
    fn the_generated_nats_config_uses_the_ports_it_advertises() {
        let dir = tempfile::tempdir().unwrap();
        let operator = dir.path().join("operator.jwt");
        std::fs::write(&operator, "{}").unwrap();
        let conf = nats_conf(
            &operator,
            "AACCOUNT",
            "ACCOUNTJWT",
            "nats://relay.example:14222",
            "wss://relay.example:14090",
            None,
        );
        assert!(conf.contains("listen: 0.0.0.0:14222"), "{conf}");
        assert!(conf.contains("port: 14090"), "{conf}");
        assert!(conf.contains("resolver: MEMORY"), "{conf}");
        assert!(conf.contains("AACCOUNT: \"ACCOUNTJWT\""), "{conf}");
        // No store: a relay must not accumulate ciphertext.
        assert!(!conf.contains("jetstream"), "{conf}");
        // Without a certificate the template must not pretend TLS is on.
        assert!(!conf.contains("tls {"), "{conf}");
        assert!(conf.contains("no_tls: true"), "{conf}");
    }

    /// With a certificate the client port and the websocket port both get a
    /// TLS block: the phone refuses `ws://`, so a relay that advertises `wss://`
    /// and listens in plaintext would hand out unusable invitations.
    #[test]
    fn a_tls_certificate_wires_both_listeners() {
        let dir = tempfile::tempdir().unwrap();
        let operator = dir.path().join("operator.jwt");
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        for path in [&operator, &cert, &key] {
            std::fs::write(path, "x").unwrap();
        }
        let conf = nats_conf(
            &operator,
            "AACCOUNT",
            "ACCOUNTJWT",
            "tls://relay.example:14222",
            "wss://relay.example:14090",
            Some((&cert, &key)),
        );
        assert!(conf.contains("tls {"), "{conf}");
        assert!(conf.contains("cert_file:"), "{conf}");
        assert!(conf.contains("key_file:"), "{conf}");
        assert!(!conf.contains("no_tls: true"), "{conf}");
        assert!(conf.contains("  tls {}"), "{conf}");
    }
}
