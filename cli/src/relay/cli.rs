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
         NATS credential. With a certificate in relay.json and an `https://`
         public URL, the API itself serves TLS; without one it stays plaintext
         and relies on a TLS terminator in front of it.

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

The certificate covers the relay's own HTTP API too. `init` records
--tls-cert/--tls-key in relay.json, and `serve` presents that certificate when
--public-url is `https://`, so one certificate serves the broker, the
websocket listener and the API. An `https://` public URL with no certificate
is the reverse-proxy deployment: `serve` stays plaintext and says so at
startup. --allow-plaintext-upstream leaves the certificate out of relay.json
for the same reason — the proxy in front is the one doing TLS.

The relay has no accounts and no database. It keeps rooms in a JSON file beside
relay.json, so a restart does not cost a re-pair, and it never stores a
credential: only hashes of the invitation and refresh tokens.

A desktop points at it with `futureos --relay URL` or
`future host --relay URL`; the invitation it prints then carries the
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
        // `serve` terminates TLS for the API too, unless the operator said a
        // TLS-terminating hop sits in front of the relay: then the listener is
        // *meant* to stay plaintext and a recorded certificate would make
        // `serve` start a second TLS layer behind the proxy.
        tls_cert: match (&args.tls_cert, args.allow_plaintext_upstream) {
            (Some(cert), false) => Some(cert.to_string_lossy().into_owned()),
            _ => None,
        },
        tls_key: match (&args.tls_key, args.allow_plaintext_upstream) {
            (Some(key), false) => Some(key.to_string_lossy().into_owned()),
            _ => None,
        },
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

    // The API terminates TLS itself whenever the file names a certificate and
    // the public URL is `https://`. Without a certificate an `https://` public
    // URL keeps working behind a TLS terminator (a reverse proxy), which is the
    // deployment the generated config has always supported.
    let tls = if config.terminates_tls() {
        let cert = config.tls_cert.as_deref().expect("terminates_tls checked");
        let key = config.tls_key.as_deref().expect("terminates_tls checked");
        Some((
            PathBuf::from(cert),
            PathBuf::from(key),
            server_tls_config(Path::new(cert), Path::new(key))?,
        ))
    } else {
        None
    };
    if tls.is_none() && config.public_url.trim_start().starts_with("https://") {
        eprintln!(
            "future relay: public URL {} is https but no certificate is configured; \
             serving plaintext and relying on a TLS terminator in front of this listener",
            config.public_url
        );
    }

    let rooms_path = config_path.with_file_name("rooms.json");
    let relay = Arc::new(Relay::new(config, Some(&rooms_path))?);
    match &tls {
        Some((cert, _, _)) => println!(
            "future relay listening on https://{listen} (cert {}, account {}, public {}, rooms {})",
            cert.display(),
            relay.account_public_key(),
            relay.config.public_url,
            relay.rooms.len()
        ),
        None => println!(
            "future relay listening on http://{listen} (account {}, public {}, rooms {})",
            relay.account_public_key(),
            relay.config.public_url,
            relay.rooms.len()
        ),
    }

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
        match tls {
            None => axum::serve(listener, app).await?,
            Some((_, _, config)) => serve_tls(listener, config, app).await?,
        }
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(())
}

/// The TLS material the relay's own listener presents.
///
/// A certificate that does not match its key is an error at startup rather than
/// at the first handshake: a relay that boots and then fails every pairing is
/// harder to diagnose than one that refuses to start.
fn server_tls_config(cert: &Path, key: &Path) -> anyhow::Result<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    // The binary installs this provider for every other TLS client; nothing else
    // in this process builds a rustls configuration before `serve` runs.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificates = CertificateDer::pem_file_iter(cert)
        .with_context(|| format!("read the certificate {}", cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parse the certificates in {}", cert.display()))?;
    if certificates.is_empty() {
        bail!("{} holds no certificate", cert.display());
    }
    let key = PrivateKeyDer::from_pem_file(key)
        .with_context(|| format!("read the private key {}", key.display()))?;
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .with_context(|| {
            format!(
                "{} and its key are not a usable pair (do they belong together?)",
                cert.display()
            )
        })?;
    // The API is JSON over HTTP/1.1. Advertising h2 here would promise a
    // protocol this listener does not need and cannot be tested against.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Accept TLS connections and serve the relay's router on each of them.
///
/// `axum::serve` only takes a plaintext `TcpListener` in 0.7, so the handshake
/// and the connection loop live here. Everything below the socket is the same
/// router `axum::serve` would run.
async fn serve_tls(
    listener: tokio::net::TcpListener,
    config: Arc<rustls::ServerConfig>,
    app: axum::Router,
) -> anyhow::Result<()> {
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // A failed accept (e.g. `EMFILE` during a burst) must not stop a
                // relay that every paired phone depends on; back off briefly
                // instead of spinning on the same condition.
                eprintln!("future relay: accept failed: {error}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let service = hyper_util::service::TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(error) => {
                    // A client that does not trust the certificate, or that
                    // scanned the port with a plaintext probe, lands here. It is
                    // ordinary traffic, not a relay failure.
                    let _ = (peer, error);
                    return;
                }
            };
            let _ =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await;
        });
    }
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
        // The proxy in front terminates TLS for the API too, so `serve` must not
        // start a second TLS layer behind it: the certificate stays out of
        // relay.json and the listener stays plaintext.
        let proxied = RelayConfig::load(&with_cert.join("relay.json")).expect("config written");
        assert!(!proxied.terminates_tls(), "{proxied:?}");
        assert_eq!(proxied.tls_cert, None);
        assert_eq!(proxied.tls_key, None);
    }

    /// Without a proxy in front, the certificate covers the relay's own HTTP
    /// API: `init` records it in relay.json so `serve` presents it, instead of
    /// leaving every deployment to configure an external TLS terminator.
    #[test]
    fn a_certificate_lets_serve_terminate_tls_itself() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("fullchain.pem");
        let key = dir.path().join("privkey.pem");
        for path in [&cert, &key] {
            std::fs::write(path, "x").unwrap();
        }
        let quiet = dir.path().join("relay");
        run(&[
            "init".to_string(),
            "--dir".to_string(),
            quiet.to_string_lossy().to_string(),
            "--public-url".to_string(),
            "https://relay.example.com".to_string(),
            "--nats-url".to_string(),
            "tls://relay.example.com:4222".to_string(),
            "--nats-ws-url".to_string(),
            "wss://relay.example.com:9090".to_string(),
            "--tls-cert".to_string(),
            cert.to_string_lossy().to_string(),
            "--tls-key".to_string(),
            key.to_string_lossy().to_string(),
        ])
        .expect("init with a certificate");
        let config = RelayConfig::load(&quiet.join("relay.json")).expect("config written");
        assert_eq!(
            config.tls_cert.as_deref(),
            Some(cert.to_string_lossy().as_ref())
        );
        assert_eq!(
            config.tls_key.as_deref(),
            Some(key.to_string_lossy().as_ref())
        );
        assert!(
            config.terminates_tls(),
            "an https public URL plus a certificate means serve terminates TLS"
        );
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

#[cfg(test)]
mod serve_tls_tests {
    use super::*;
    use crate::relay::server::{router, Relay};
    use rustls::pki_types::pem::PemObject;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A CA plus the leaf it issued for `127.0.0.1` / `localhost`, as PEM — the
    /// same shape `--tls-cert/--tls-key` take, so the test drives the shipping
    /// code path rather than a hand-built `ServerConfig`.
    const CA_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIDITCCAgmgAwIBAgIUVUBkun4adpEqs3BDaX466CvcomYwDQYJKoZIhvcNAQEL\nBQAwJzElMCMGA1UEAwwcRnV0dXJlT1MgUmVsYXkgU2VydmUgVGVzdCBDQTAeFw0y\nNjEwMjEwMzM5MjRaFw0zNjEwMTgwMzM5MjRaMCcxJTAjBgNVBAMMHEZ1dHVyZU9T\nIFJlbGF5IFNlcnZlIFRlc3QgQ0EwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEK\nAoIBAQCUYIK9PQvLLRxaU/r3olcE7PU82xhQvlV+19j/4FUjG1mHOGAHE0o3W4XK\nn7xhep8ZnlEifzDg+oyFyASW52e6K0SkILfzq9zdJPBMJI0mrEcN4nAZCLShwskH\npKB78gfW97/sgLipsbTFY0gq5+5pA5EO7AQAlvRnjaYNd2THDa/gbLZezDgtCGEa\n+pgEqWB7k2iHhrAotKRnefu2Uze7vYBNJZ9EHu+D7gs0XkXaccED9nEZxrMv61+z\nuiBw1m4wpIPF75/7/S8iFz4ie7I9k1E11hpdM1d9LO2LNs+mpDw1+G3m8QVOiFUo\nOEENXaNYzq3zI0OoHxhZka/9RCZvAgMBAAGjRTBDMBIGA1UdEwEB/wQIMAYBAf8C\nAQAwDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBRBHHoUWyEHNNxtuqPPLA8IkmGd\nHTANBgkqhkiG9w0BAQsFAAOCAQEAbOoK8J5+mqhSrMegMvk3E/Iev2rsJaw3m/kd\nB4rjZKsGndXgIVODPqiMuZ5DjDkqVTxgKPqGIyKbNC93tBEjpcRq8bRoV1LPN5Gh\nW4aV8fdPmVemWsqC76jSsDeGI0g92h70+9Ur7wur2HvDFa19vKg5h84e8oli0Aht\n9SxfsYB2Mm1hlDMKm5/shAw7HZOotoRWXeXJeWEWWdpqnuCCEhCqWhRe58dZ87gE\nPbG7i2yX9HuHwc+FdOk5ijBkyAun2NUuZFYSBMVKS2sIFAV+FGaquapFXKVgz9Ok\nRztdDYOVrmHDwlXGMLoU09JKl5qgOJC1rSY/1svsKNaQ1TaMJQ==\n-----END CERTIFICATE-----\n";
    const SERVER_CERT_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIDXDCCAkSgAwIBAgIUJQhnaQS4NNRhKCtWrwPyOecx4jswDQYJKoZIhvcNAQEL\nBQAwJzElMCMGA1UEAwwcRnV0dXJlT1MgUmVsYXkgU2VydmUgVGVzdCBDQTAeFw0y\nNjEwMjEwMzM5MjRaFw0yNzExMjIwMzM5MjRaMBQxEjAQBgNVBAMMCTEyNy4wLjAu\nMTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAOie6tXf75GU/pJSJD8v\nK326UImyLa0SroFqPkkg4Jl/VL309sMOM3Mfa6JPqBaPbbIcjSTc+Fb5XObJRPxa\nonzarxIpiH1hDEitmwhLAmVGFu2MsV8LCFVlsHd4KqSiaLcz6dja5Kyo3yJ8/Ahc\nD8sWfC04Lrh/oE8PN06DFce7Hd/VEKIfC/B0dXqE2cOmEmHm5IpO4E6MvJqgH/d5\nR/TRkvzFqIBWt78F1QtMgZnAWE4lMtg+bH9mbHhonnsBw+7ipAO/Tm4rGH+tzDAz\nm78iU7Q82ldsHhCFd5w4JinLgnoRXPY5deHIOJb8CBR48TOaEcXo3CkTuc0WSMDt\nEQUCAwEAAaOBkjCBjzAaBgNVHREEEzARhwR/AAABgglsb2NhbGhvc3QwEwYDVR0l\nBAwwCgYIKwYBBQUHAwEwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCBaAwHQYD\nVR0OBBYEFMXdY+2joJg8syDdJJ/CZyFSdP79MB8GA1UdIwQYMBaAFEEcehRbIQc0\n3G26o88sDwiSYZ0dMA0GCSqGSIb3DQEBCwUAA4IBAQCEVkJgEuW5c4V8Ue12Poz4\nXVnaxD6Z0fnDB8EZEgez8v5dx55MpNMfYHVdmdZ2EwBEoPTEI8uxnByMrKVNo7g6\nKUzaZ52bPbVfjZ7JW4jxb+kZUgZ+eUaZSfLLEQBviiHTAs3vbzSz9nVgC9RmGhmc\nJduiIXNzBLqmHOl2vyhpvcS8G/zV1LnI4zihbfb/793cqLGu6k7rkxmgXUVmr0C6\n/iVtDwYO6rFr0lJ/aqBh3IRS45q0Q43YcdEm4ZS0P+XeVCztYiEDx9gKw/yf4jyE\ntmZoICjLkbLFjg21hG9GGskjZaalq3GeRXtwiUM/BV4aQozhhWURollhX8HqWpSE\n-----END CERTIFICATE-----\n";
    const SERVER_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDonurV3++RlP6S\nUiQ/Lyt9ulCJsi2tEq6Baj5JIOCZf1S99PbDDjNzH2uiT6gWj22yHI0k3PhW+Vzm\nyUT8WqJ82q8SKYh9YQxIrZsISwJlRhbtjLFfCwhVZbB3eCqkomi3M+nY2uSsqN8i\nfPwIXA/LFnwtOC64f6BPDzdOgxXHux3f1RCiHwvwdHV6hNnDphJh5uSKTuBOjLya\noB/3eUf00ZL8xaiAVre/BdULTIGZwFhOJTLYPmx/Zmx4aJ57AcPu4qQDv05uKxh/\nrcwwM5u/IlO0PNpXbB4QhXecOCYpy4J6EVz2OXXhyDiW/AgUePEzmhHF6NwpE7nN\nFkjA7REFAgMBAAECggEAEgGjKfUQpVNENW1YUu/CzROsAkyhjoEgsvHXam+6Ewex\nNpcMtuKVhvbKrSC/YSevFRdysYTNUGYzE8ZGqcYWR9XhsWU3pITrn4CYvJaZ098O\nDUtTu4iuF3Ysp8LMgjv0ISJKmreu9dKImzHTjI4bxtuxtwpfS3bNoPELle89s10Z\n1Anr5gDdGeW3Hxk/RnU++KMVSObDS1VZjSJQ+HItYuJyevX9VGmcIQfgCY49+G0o\na4eVTNbetCmdPTuKnOgIjBmtTsgXcMCG0e8wAVvyWoYqgIHXlQDU6pohH3l7UTH8\nBVhwuSLqSLk3Ja6SO4RYH5HmrtROvNcOPs29iEN2TQKBgQD5WGPkztGdQaGLYU35\nUBA6Wq/s3mHcCsMf3W4bhhJ/89nvWEBGS5PPxhJ7sWTPDoelQytTampMGGPcTM8R\nPlNPrIdyCbo2dV9cYBSeqjea34fiFs5XDsyG/WX1SxOjfnwFS61uDTkO6jqPinhM\nZVITuK7oF7sIZxgSFoUt0Pl12wKBgQDu1EJ+8TBPToyeQvq0qg8vsFB0kaHRCi0u\nmpWTd8CdQh+wSdCcETAUzUy3SsCC7FtTLbHR4jAxz4DaNSX/qkFHfddQbuxOE4sD\nGhj0K2fBxUklbpWeCXkXOrwCaaMiGU4dllJEhGVojLKONSbVqxXtezhb2r0QTmHW\nwIJAiD/6nwKBgQDVrTudvua+3plzeiV5+3SMrV6JnGE8UjS4v+c1ywVbhTtD8trR\n4r5+lX0NZw2+qvLvWspsJ4uEPnqFNYVVf5Whd3Zy8YcUWC0qWmQRPEl+fQRTabvV\nuybmn4yaUoY325KobxvmbYpyD6OzkrRYs5NMtz2cQ4nSP5bPGWFzy9ubdQKBgEVR\n2pMZ0lnShMIS0s3JZmsDE/chpgQUnhSezbdl1MRQQv5aDnCeFISVKiUxTzsw+zjj\nVG+E5VQv1EiBkKvqcNQdVHeGGpR226xk0MVeK9l0N8zjkg2y1jDF3XgdzTnCeoff\n77TvcLetzs2z47e01sJ9N1iB3spy1bcNXUD5ywglAoGBALeyldkGLqObYovrqkBx\nWR/g9n5QAfbJv0Ww+Vb/FEBBKOHi/gd6UamoVsVIC0qTyRxSC50onLHW1raR+qgk\nAlVzRgbakmvP+RZtnWCo3GdAfUEKj5WvJGGnMPTw/j2rHgsX3ANQVKxx0OpFln8r\nCpd7bnByNNjOfkFH8k6TqSF9\n-----END PRIVATE KEY-----\n";

    /// Write the fixture into `dir` and return `(cert, key)`.
    fn write_certificate(dir: &std::path::Path) -> (PathBuf, PathBuf) {
        let cert = dir.join("fullchain.pem");
        let key = dir.join("privkey.pem");
        std::fs::write(&cert, SERVER_CERT_PEM).expect("write the certificate");
        std::fs::write(&key, SERVER_KEY_PEM).expect("write the key");
        (cert, key)
    }

    fn relay_config(
        public_url: String,
        cert: Option<PathBuf>,
        key: Option<PathBuf>,
    ) -> RelayConfig {
        // A real account seed: `Relay::new` refuses anything else.
        let account = nkeys::KeyPair::new_account();
        RelayConfig {
            listen: "127.0.0.1:0".into(),
            public_url,
            nats_url: "tls://127.0.0.1:4222".into(),
            nats_ws_url: "wss://127.0.0.1:9090".into(),
            account_seed: account.seed().expect("account seed"),
            jwt_ttl_seconds: 300,
            invitation_ttl_seconds: 300,
            room_ttl_seconds: 7_776_000,
            subs_limit: -1,
            payload_limit: 1_048_576,
            http_timeout_seconds: 15,
            tls_cert: cert.map(|path| path.to_string_lossy().into_owned()),
            tls_key: key.map(|path| path.to_string_lossy().into_owned()),
        }
    }

    /// One HTTPS request over a fresh TLS connection. Returns the status line
    /// and body, so a caller can assert either successful service or refusal.
    async fn https_get(port: u16, roots: Option<&rustls::RootCertStore>) -> Result<String, String> {
        let config = match roots {
            Some(roots) => rustls::ClientConfig::builder()
                .with_root_certificates(roots.clone())
                .with_no_client_auth(),
            None => {
                let mut empty = rustls::RootCertStore::empty();
                empty.add_parsable_certificates([]);
                rustls::ClientConfig::builder()
                    .with_root_certificates(empty)
                    .with_no_client_auth()
            }
        };
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|error| error.to_string())?;
        let name = rustls::pki_types::ServerName::try_from("localhost")
            .expect("server name")
            .to_owned();
        let mut stream = connector
            .connect(name, stream)
            .await
            .map_err(|error| error.to_string())?;
        stream
            .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .map_err(|error| error.to_string())?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .map_err(|error| error.to_string())?;
        Ok(String::from_utf8_lossy(&response).into_owned())
    }

    fn roots_from(pem: &str) -> rustls::RootCertStore {
        let mut roots = rustls::RootCertStore::empty();
        let certificates: Vec<_> =
            rustls::pki_types::CertificateDer::pem_slice_iter(pem.as_bytes())
                .collect::<Result<_, _>>()
                .expect("fixture CA");
        assert!(!certificates.is_empty());
        roots.add_parsable_certificates(certificates);
        roots
    }

    /// The relay's own listener really speaks TLS: a client that trusts the
    /// relay's CA completes a request, and the same client fails against a
    /// store that does not contain it. This is the capability a plaintext-only
    /// `serve` left to a reverse proxy.
    #[tokio::test]
    async fn serve_terminates_tls_with_the_configured_certificate() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (cert, key) = write_certificate(dir.path());
        let config = relay_config(
            "https://localhost".into(),
            Some(cert.clone()),
            Some(key.clone()),
        );
        assert!(config.terminates_tls());

        let tls = server_tls_config(&cert, &key).expect("TLS configuration");
        let relay = Arc::new(Relay::new(config, None).expect("in-memory relay"));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("address").port();
        let server = tokio::spawn(serve_tls(listener, tls, router(relay)));

        // A client that trusts the private CA reaches the API.
        let roots = roots_from(CA_PEM);
        let response = https_get(port, Some(&roots))
            .await
            .expect("the relay answers over TLS");
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "unexpected response: {response}"
        );
        assert!(response.contains("\"ok\":true"), "{response}");

        // Without that root the handshake is refused: the listener is verified
        // TLS, not "TLS-shaped". Both halves together are what proves the
        // certificate is actually used.
        let error = https_get(port, None)
            .await
            .expect_err("a private root that is not trusted cannot verify");
        let lower = error.to_lowercase();
        assert!(
            lower.contains("certificate") || lower.contains("unknown issuer"),
            "expected a certificate failure, got: {error}"
        );

        server.abort();
    }

    /// A certificate that does not match its key is refused at startup, not at
    /// the first pairing attempt.
    #[test]
    fn a_mismatched_key_is_refused_before_the_listener_starts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (cert, _) = write_certificate(dir.path());
        // A second, unrelated key.
        let other = dir.path().join("other.pem");
        std::fs::write(
            &other,
            "-----BEGIN PRIVATE KEY-----\nnot a key\n-----END PRIVATE KEY-----\n",
        )
        .expect("write the unusable key");
        let error = server_tls_config(&cert, &other)
            .expect_err("an unusable key pair must not boot")
            .to_string();
        assert!(
            error.contains("privkey") || error.contains("other.pem") || error.contains("key"),
            "{error}"
        );
    }
}
