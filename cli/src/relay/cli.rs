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
                     [--tls-cert PEM --tls-key PEM]
                     [--allow-plaintext-upstream] [--force]
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

The listeners and the URLs they advertise have to agree. A `tls://` or
`wss://` endpoint promises TLS, so `init` needs --tls-cert/--tls-key for it and
refuses to write a plaintext listener advertised as TLS; a certificate with a
plaintext endpoint is refused for the same reason. Only when the relay sits
behind something else that terminates TLS — a reverse proxy, a load balancer —
pass --allow-plaintext-upstream to skip that check: the relay then listens in
plaintext and still advertises `tls://`/`wss://`, which is correct only because
the hop in front of it is the one doing TLS.

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
    allow_plaintext_upstream: bool,
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
        allow_plaintext_upstream: false,
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
            "--allow-plaintext-upstream" => parsed.allow_plaintext_upstream = true,
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
    check_transport(&parsed)?;
    Ok(parsed)
}

/// The scheme of a `scheme://…` URL, lower-case, or `""` when it has none.
fn url_scheme(url: &str) -> String {
    url.split("://")
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Whether clients reading this endpoint will attempt a TLS handshake.
fn is_tls_scheme(url: &str) -> bool {
    matches!(url_scheme(url).as_str(), "tls" | "wss")
}

/// Hold the advertised scheme and the generated listener to the same transport.
///
/// `init` copies `--nats-url`/`--nats-ws-url` into every credential response, so
/// the scheme there is a promise to the clients. The generated `nats.conf` only
/// speaks TLS when it was handed a certificate; `tls://` with no certificate
/// therefore produces a plaintext listener that the desktop refuses (it requires
/// TLS unless the pairing is a relay's `nats://`) and a `wss://` endpoint the
/// phone cannot open — a deployment that looks configured and cannot pair. The
/// reverse is the same mistake from the other side.
///
/// The one topology this cannot see is a TLS terminator in front of the relay
/// (a reverse proxy, a load balancer, a service mesh). There the listener is
/// meant to stay plaintext while the advertised URL is not, so the check is
/// skipped only when the operator says so with `--allow-plaintext-upstream`.
fn check_transport(args: &InitArgs) -> anyhow::Result<()> {
    if args.allow_plaintext_upstream {
        return Ok(());
    }
    let endpoints = [
        ("--nats-url", args.nats_url.as_str()),
        ("--nats-ws-url", args.nats_ws_url.as_str()),
    ];
    let mut tls: Vec<(&str, &str)> = Vec::new();
    let mut plaintext: Vec<(&str, &str)> = Vec::new();
    for (flag, url) in endpoints {
        if is_tls_scheme(url) {
            tls.push((flag, url));
        } else {
            plaintext.push((flag, url));
        }
    }
    if args.tls_cert.is_none() {
        if let Some((flag, url)) = tls.first() {
            bail!(
                "{flag} {url} declares TLS, but no certificate was given: pass --tls-cert PEM --tls-key PEM so the listener can serve it, use a plaintext endpoint (nats:// / ws://) for a relay on a local network, or pass --allow-plaintext-upstream if a TLS-terminating proxy sits in front of this relay"
            );
        }
        return Ok(());
    }
    if let Some((flag, url)) = plaintext.first() {
        bail!(
            "{flag} {url} is plaintext, but --tls-cert/--tls-key were given: `init` cannot make that endpoint speak TLS, so use tls:// / wss:// there, drop the certificate (a relay on a local network needs no TLS), or pass --allow-plaintext-upstream if a TLS-terminating proxy sits in front of this relay"
        );
    }
    Ok(())
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
    // The websocket listener needs the certificate **in its own block**:
    // nats-server does not inherit the top-level `tls` for websocket traffic.
    // An empty `tls {}` there is accepted at startup and then fails every
    // handshake with `tls: no certificates configured`, and omitting the block
    // while `wss://` is advertised is refused outright ("websocket requires TLS
    // configuration"). Either way the phone cannot pair, so repeat the paths.
    let ws_tls = match tls {
        Some((cert, key)) => format!(
            "  tls {{\n    cert_file: \"{}\"\n    key_file: \"{}\"\n  }}\n",
            absolute(cert),
            absolute(key)
        ),
        None => "  # Terminate TLS here (cert_file/key_file) for a public deployment: the\n\
             \x20 # shipping phone client refuses anything but wss://.\n\
             \x20 no_tls: true\n"
            .to_string(),
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
         # nats-server's HTTP monitoring endpoint is off by default: the relay\n\
         # itself never reads it, and a fixed port would keep a second relay on\n\
         # this machine from starting (`bind: Only one usage of each socket`).\n\
         # Uncomment for stats on loopback: `http: 127.0.0.1:8222`.\n\
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
        // No monitoring listener: the relay never reads it, and a fixed port
        // stops a second relay on the same machine from starting. The hint is
        // commented out, so the default file has no active `http:` directive.
        assert!(
            !conf
                .lines()
                .any(|line| line.trim_start().starts_with("http:")),
            "{conf}"
        );
        assert!(conf.contains("http: 127.0.0.1:8222"), "{conf}");
    }

    /// A `tls://`/`wss://` endpoint is a promise to every client that reads the
    /// credential response. Without a certificate the generated listener stays
    /// plaintext, so `init` must refuse rather than write a relay nobody can
    /// pair with — the desktop requires TLS and the phone requires `wss://`.
    #[test]
    fn a_tls_scheme_without_a_certificate_is_refused() {
        for (flag, url) in [
            ("--nats-url", "tls://relay.example.com:4222"),
            ("--nats-ws-url", "wss://relay.example.com:9090"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().to_string_lossy().to_string();
            let error = run(&[
                "init".to_string(),
                "--dir".to_string(),
                path,
                flag.to_string(),
                url.to_string(),
            ])
            .expect_err("a TLS scheme needs a certificate")
            .to_string();
            assert!(error.contains(url), "{flag} is not named: {error}");
            assert!(error.contains("--tls-cert"), "no way out given: {error}");
            // Nothing is half-written: the check runs before the directory work.
            assert!(!dir.path().join("relay.json").exists());
        }
    }

    /// The same mismatch from the other side: a certificate makes both listeners
    /// speak TLS, so an endpoint advertised as plaintext would be wrong too.
    #[test]
    fn a_certificate_with_a_plaintext_scheme_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        for path in [&cert, &key] {
            std::fs::write(path, "x").unwrap();
        }
        let error = run(&[
            "init".to_string(),
            "--dir".to_string(),
            dir.path().to_string_lossy().to_string(),
            "--nats-url".to_string(),
            "nats://relay.example.com:4222".to_string(),
            "--nats-ws-url".to_string(),
            "wss://relay.example.com:9090".to_string(),
            "--tls-cert".to_string(),
            cert.to_string_lossy().to_string(),
            "--tls-key".to_string(),
            key.to_string_lossy().to_string(),
        ])
        .expect_err("a plaintext endpoint with a certificate is a mismatch")
        .to_string();
        assert!(error.contains("nats://relay.example.com:4222"), "{error}");
        assert!(error.contains("--allow-plaintext-upstream"), "{error}");
    }

    /// A TLS terminator in front of the relay is a real deployment, so the
    /// operator can say so: the check is skipped and the plaintext listener is
    /// written while the advertised URL keeps its own scheme.
    #[test]
    fn allow_plaintext_upstream_accepts_either_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        for path in [&cert, &key] {
            std::fs::write(path, "x").unwrap();
        }
        // No certificate, TLS advertised: the proxy in front does the TLS.
        let behind_proxy = dir.path().join("behind-proxy");
        run(&[
            "init".to_string(),
            "--dir".to_string(),
            behind_proxy.to_string_lossy().to_string(),
            "--nats-url".to_string(),
            "tls://relay.example.com:4222".to_string(),
            "--nats-ws-url".to_string(),
            "wss://relay.example.com:9090".to_string(),
            "--allow-plaintext-upstream".to_string(),
        ])
        .expect("the flag opts out of the scheme check");
        let config = RelayConfig::load(&behind_proxy.join("relay.json")).expect("config written");
        assert_eq!(config.nats_url, "tls://relay.example.com:4222");
        let conf = std::fs::read_to_string(behind_proxy.join("nats.conf")).unwrap();
        assert!(
            conf.contains("no_tls: true"),
            "the listener stays plaintext"
        );

        // Certificate present, plaintext advertised: the same opt-out covers it.
        let with_cert = dir.path().join("with-cert");
        run(&[
            "init".to_string(),
            "--dir".to_string(),
            with_cert.to_string_lossy().to_string(),
            "--tls-cert".to_string(),
            cert.to_string_lossy().to_string(),
            "--tls-key".to_string(),
            key.to_string_lossy().to_string(),
            "--allow-plaintext-upstream".to_string(),
        ])
        .expect("the flag opts out of the scheme check");
        assert!(with_cert.join("nats.conf").exists());
    }

    /// With a certificate each listener carries its certificate **in its own
    /// block**: nats-server does not let websocket inherit the top-level `tls`,
    /// so an empty `tls {}` there starts up and then fails every handshake with
    /// `tls: no certificates configured` — and dropping the block while
    /// advertising `wss://` is refused ("websocket requires TLS configuration").
    /// Either way the phone cannot pair, which is what this pins.
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
        assert!(!conf.contains("no_tls: true"), "{conf}");
        // Two `tls` blocks, each naming the same pair: one for the client port,
        // one inside `websocket`.
        assert_eq!(conf.matches("\ntls {").count(), 1, "{conf}");
        let websocket = conf.split("websocket {").nth(1).expect("a websocket block");
        assert!(websocket.contains("tls {"), "{websocket}");
        assert!(websocket.contains("cert_file:"), "{websocket}");
        assert!(websocket.contains("key_file:"), "{websocket}");
        assert!(websocket.contains("cert.pem"), "{websocket}");
        assert!(websocket.contains("key.pem"), "{websocket}");
        // An empty block is the bug this pins: it configures TLS for websocket
        // without a certificate.
        assert!(!conf.contains("tls {}"), "{conf}");
    }
}
