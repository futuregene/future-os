//! Hostname resolution for this process's HTTP clients.
//!
//! A binary linked against musl carries musl's resolver, which reads two fixed
//! paths — `/etc/hosts` and `/etc/resolv.conf` — with no environment override
//! and no NSS. On a system where neither exists (a minimal rootfs, a container
//! built without `/etc`, an appliance image that never had one) a hostname
//! lookup then fails after a five-second stall, and no configuration elsewhere
//! in the process can help. Measured in a `chroot` with no `/etc` at all:
//! `getaddrinfo` returns `EAI_AGAIN` ("Try again") after 5.0 s, and `localhost`
//! fails too. With only `/etc/hosts` the same 5.0 s stall remains for every DNS
//! name; with only `/etc/resolv.conf` `localhost` fails immediately with
//! `EAI_NONAME`.
//!
//! This module is the one place this process turns a hostname into addresses
//! for HTTP. [`crate::client_builder`] attaches it, so every client built
//! through this crate — the Agent, the application host, the CLI, the TUI —
//! resolves through it instead of `getaddrinfo`. The CLI's gRPC path
//! (`tonic`, which does not go through `reqwest`) reuses [`resolve`] through
//! its own connector. What is **not** covered is the NATS client: `async-nats`
//! resolves inside itself (`ToServerAddrs::socket_addrs` calls
//! `tokio::net::lookup_host`) and exposes no hook to replace that.
//!
//! The default path is unchanged on purpose. When `/etc/resolv.conf` exists and
//! no environment variable says otherwise, [`Plan::detect`] returns
//! [`Plan::System`]: no resolver is attached and every lookup goes through
//! `getaddrinfo`, exactly as before this module existed. A custom resolver is
//! built only for the two cases the guide names — a missing default file, or an
//! explicit `FUTURE_RESOLV_CONF` / `FUTURE_DNS_SERVER`. See
//! `docs/guide/dns-resolution.md`.

mod config;
mod query;

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};

use config::{Plan, DNS_SERVER_ENV, RESOLV_CONF_ENV};

/// Whether this process resolves through a configured nameserver rather than
/// `getaddrinfo`. Entry points that build their own connectors (the CLI's gRPC
/// endpoint) ask this before attaching a resolver.
pub fn is_custom() -> bool {
    matches!(plan(), Plan::Custom(_))
}

/// Render a failed request with its causes, because `reqwest::Error`'s own
/// `Display` prints only its kind ("error sending request for url (…)") and
/// the resolver's explanation sits one level down in the source chain.
pub fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Resolve `host` to every address it has, in the order they should be tried.
///
/// Returns a `'static` future so a long-lived client (reqwest's resolver trait,
/// tonic's connector) can own the call.
pub fn resolve(
    host: &str,
    port: u16,
) -> impl std::future::Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'static {
    resolve_with(plan(), host, port)
}

fn resolve_with(
    plan: &'static Plan,
    host: &str,
    port: u16,
) -> impl std::future::Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'static {
    let host = host.to_string();
    async move {
        match plan {
            // `getaddrinfo`, i.e. exactly what this process did before.
            Plan::System => {
                let addresses = tokio::net::lookup_host((host.as_str(), port)).await?;
                Ok(addresses.collect())
            }
            Plan::Custom(config) => resolve_configured(config, &host, port).await,
        }
    }
}

/// Resolve the host of a TCP endpoint before a client connects, so a resolution
/// failure is reported as a resolution failure.
///
/// `tonic` renders any connector error as `transport error` and keeps the cause
/// one level down, so without this the CLI would print the same four words for
/// "no nameserver anywhere" and for "connection refused" — the reason this
/// whole module exists. It resolves **before** the connect and does not replace
/// the resolver handed to the connector: the addresses are not cached, so a
/// failed connect falls back to the connector's own attempt.
///
/// A system plan is left alone (nothing to add to `getaddrinfo`), and so is
/// anything that is not a TCP endpoint — `auto`, a unix socket path, a named
/// pipe.
pub async fn preflight(endpoint: &str) -> Result<(), String> {
    let plan = plan();
    if matches!(plan, Plan::System) {
        return Ok(());
    }
    let Some((host, port)) = tcp_host_port(endpoint) else {
        return Ok(());
    };
    resolve_with(plan, &host, port)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The host and port of a TCP endpoint, or `None` when the endpoint is not one.
fn tcp_host_port(endpoint: &str) -> Option<(String, u16)> {
    if endpoint.eq_ignore_ascii_case("auto") {
        return None;
    }
    let (scheme, rest) = match endpoint.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, endpoint),
    };
    // A unix socket path or a named pipe is not a host name.
    if rest.contains('/') || rest.contains('\\') {
        return None;
    }
    let default_port = if scheme == Some("https") { 443 } else { 80 };
    match rest.rsplit_once(':') {
        Some((host, port)) => {
            let port = port.parse().ok()?;
            let host = host.trim_start_matches('[').trim_end_matches(']');
            (!host.is_empty()).then(|| (host.to_string(), port))
        }
        None => (!rest.is_empty()).then(|| (rest.to_string(), default_port)),
    }
}

fn plan() -> &'static Plan {
    static PLAN: OnceLock<Plan> = OnceLock::new();
    PLAN.get_or_init(Plan::detect)
}

/// Attach this process's resolver to an async client builder.
pub(crate) fn attach(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    attach_with(builder, plan())
}

/// Attach this process's resolver to a blocking client builder.
pub(crate) fn attach_blocking(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    match resolver_for(plan()) {
        None => builder,
        Some(resolver) => builder.dns_resolver(resolver),
    }
}

fn attach_with(builder: reqwest::ClientBuilder, plan: &'static Plan) -> reqwest::ClientBuilder {
    match resolver_for(plan) {
        None => builder,
        Some(resolver) => builder.dns_resolver(resolver),
    }
}

fn resolver_for(plan: &'static Plan) -> Option<Arc<Resolver>> {
    match plan {
        Plan::System => None,
        Plan::Custom(_) => Some(Arc::new(Resolver(plan))),
    }
}

/// The `reqwest` side of [`resolve`]. It holds the plan it was built for rather
/// than reading the global one: a client built from one configuration must keep
/// using it, and a test that supplies a plan must not end up exercising the
/// machine's own `/etc`.
struct Resolver(&'static Plan);

impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let plan = self.0;
        Box::pin(async move {
            // Port 0: reqwest replaces it with the URL's port (or the scheme's
            // default), which is where the port belongs.
            let addresses = resolve_with(plan, &host, 0)
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?;
            let addresses: reqwest::dns::Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

async fn resolve_configured(
    config: &config::Config,
    host: &str,
    port: u16,
) -> io::Result<Vec<SocketAddr>> {
    // An address literal is not a lookup: `getaddrinfo` answers it from the
    // parse alone, and so does this.
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(address, port)]);
    }

    // The hosts files come first, as they do in libc: a name listed there must
    // not be sent to a nameserver, and on a machine with no resolv.conf they
    // are the only names that resolve.
    for path in config.hosts_files() {
        if let Some(addresses) = config::hosts_lookup(path, host) {
            if !addresses.is_empty() {
                return Ok(addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, port))
                    .collect());
            }
        }
    }

    // A configuration that could not be read fails here, before a query is
    // sent: reporting "no answer" for a file nobody opened would send the user
    // looking for a network problem that does not exist.
    if let Some(problem) = config.problem() {
        return Err(failure(host, problem, config, io::ErrorKind::NotFound));
    }

    let mut failures = Vec::new();
    for candidate in config.search_candidates(host) {
        match query::lookup(config, &candidate).await {
            Ok(reply) if !reply.addresses.is_empty() => {
                return Ok(reply
                    .addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, port))
                    .collect());
            }
            Ok(reply) if reply.truncated => failures.push(format!(
                "{candidate}: the reply was truncated and this resolver does not retry over TCP"
            )),
            Ok(_) => failures.push(format!("{candidate}: no address record")),
            Err(error) => failures.push(format!("{candidate}: {error}")),
        }
    }
    let reason = if config.nameservers().is_empty() {
        "this process has no nameserver to ask".to_string()
    } else {
        format!(
            "asked {} ({} attempt(s), {}s each): {}",
            config
                .nameservers()
                .iter()
                .map(SocketAddr::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            config.attempts(),
            config.timeout().as_secs(),
            failures.join("; ")
        )
    };
    Err(failure(host, &reason, config, io::ErrorKind::NotFound))
}

/// One failure sentence: what failed, what this process consulted, and what the
/// user can do about it. The guidance is part of the contract — a static musl
/// build has no way to discover a configuration it was not told about.
fn failure(host: &str, reason: &str, config: &config::Config, kind: io::ErrorKind) -> io::Error {
    io::Error::new(
        kind,
        format!(
            "cannot resolve {host:?}: {reason}. Resolution in this process: {}. \
             Set {DNS_SERVER_ENV}=1.1.1.1 (or 1.1.1.1:53), set \
             {RESOLV_CONF_ENV}=/path/to/resolv.conf, add the name to one of the \
             hosts files above, or use an IP address instead of a host name. \
             A static musl build reads only these files and cannot use NSS \
             modules (mDNS, LDAP, systemd-resolved).",
            config.describe()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::config::Inputs;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// A UDP nameserver answering every A/AAAA question with `addresses`.
    ///
    /// The thread is detached on purpose: it dies with the test process, and a
    /// join handle would have to be threaded through every assertion.
    fn spawn_nameserver(addresses: Vec<IpAddr>) -> SocketAddr {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 512];
            while let Ok((length, peer)) = socket.recv_from(&mut buffer) {
                let request = &buffer[..length];
                if request.len() < 17 {
                    continue;
                }
                let qtype = u16::from_be_bytes([request[length - 4], request[length - 3]]);
                let records: Vec<(u16, Vec<u8>)> = addresses
                    .iter()
                    .filter_map(|address| match (address, qtype) {
                        (IpAddr::V4(v4), query::TYPE_A) => {
                            Some((query::TYPE_A, v4.octets().to_vec()))
                        }
                        (IpAddr::V6(v6), query::TYPE_AAAA) => {
                            Some((query::TYPE_AAAA, v6.octets().to_vec()))
                        }
                        _ => None,
                    })
                    .collect();
                let mut reply = Vec::with_capacity(64);
                reply.extend_from_slice(&request[0..2]);
                reply.extend_from_slice(&0x8180u16.to_be_bytes()); // QR|RD|RA, NOERROR
                reply.extend_from_slice(&1u16.to_be_bytes()); // one question
                reply.extend_from_slice(&(records.len() as u16).to_be_bytes());
                reply.extend_from_slice(&[0, 0, 0, 0]);
                reply.extend_from_slice(&request[12..]); // the question, verbatim
                for (rtype, rdata) in &records {
                    reply.extend_from_slice(&[0xc0, 0x0c]); // owner: the question's name
                    reply.extend_from_slice(&rtype.to_be_bytes());
                    reply.extend_from_slice(&1u16.to_be_bytes()); // class IN
                    reply.extend_from_slice(&60u32.to_be_bytes()); // ttl
                    reply.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
                    reply.extend_from_slice(rdata);
                }
                let _ = socket.send_to(&reply, peer);
            }
        });
        address
    }

    fn config_with(
        resolv_conf: Option<&str>,
        dns_server: Option<&str>,
        prefix: Option<&str>,
        files: &[(&str, &str)],
    ) -> config::Config {
        let mut map: HashMap<PathBuf, String> = HashMap::new();
        for (path, contents) in files {
            map.insert(PathBuf::from(path), (*contents).to_string());
        }
        let exists = |path: &Path| map.contains_key(path);
        let read = |path: &Path| {
            map.get(path)
                .cloned()
                .or_else(|| std::fs::read_to_string(path).ok())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        };
        let inputs = Inputs {
            dns_server,
            resolv_conf,
            prefix,
            exists: &exists,
            read: &read,
        };
        match Plan::detect_with(&inputs) {
            Plan::Custom(config) => *config,
            Plan::System => panic!("expected a custom plan"),
        }
    }

    /// A plan that outlives the call, as the clients that hold one require.
    fn leaked(config: config::Config) -> &'static Plan {
        Box::leak(Box::new(Plan::Custom(Box::new(config))))
    }

    /// The whole path a failed lookup takes: a nameserver that answers, the
    /// resolver picking the A record out of the reply, and the port the caller
    /// asked for being the one returned.
    #[tokio::test]
    async fn a_configured_nameserver_is_asked_and_its_answer_returned() {
        let expected: IpAddr = "203.0.113.7".parse().unwrap();
        let nameserver = spawn_nameserver(vec![expected]);
        let mut files: Vec<(&str, &str)> = Vec::new();
        let resolv = format!("nameserver {nameserver}\n");
        files.push(("/tmp/does-not-exist/resolv.conf", resolv.as_str()));
        let config = config_with(Some("/tmp/does-not-exist/resolv.conf"), None, None, &files);
        let addresses = resolve_configured(&config, "api.internal", 443)
            .await
            .unwrap();
        assert_eq!(addresses, vec![SocketAddr::new(expected, 443)]);
    }

    /// A hosts entry is answered locally — the case a machine without
    /// `/etc/resolv.conf` still needs, and the reason the hosts candidates are
    /// not skipped when a nameserver is configured.
    #[tokio::test]
    async fn a_hosts_entry_wins_over_the_nameserver() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        std::fs::write(dir.path().join("etc/hosts"), "10.5.5.5 api.internal\n").unwrap();
        let nameserver = spawn_nameserver(vec!["203.0.113.7".parse().unwrap()]);
        let config = config_with(
            None,
            Some(&nameserver.to_string()),
            dir.path().to_str(),
            &[],
        );
        let addresses = resolve_configured(&config, "api.internal", 80)
            .await
            .unwrap();
        assert_eq!(
            addresses,
            vec![SocketAddr::from(([10, 5, 5, 5], 80))],
            "an address in the hosts file must not be sent to a nameserver"
        );
    }

    #[tokio::test]
    async fn an_address_literal_needs_no_configuration_at_all() {
        let config = config_with(None, Some("127.0.0.1:9"), None, &[]);
        assert_eq!(
            resolve_configured(&config, "10.1.2.3", 8080).await.unwrap(),
            vec![SocketAddr::from(([10, 1, 2, 3], 8080))]
        );
        assert_eq!(
            resolve_configured(&config, "[2001:db8::1]", 8080)
                .await
                .unwrap(),
            vec!["[2001:db8::1]:8080".parse::<SocketAddr>().unwrap()]
        );
    }

    /// The message a user sees when nothing can answer. It has to name the
    /// files that were checked, the two ways to configure this, and the NSS
    /// limitation — otherwise the next person debugs the network instead of
    /// the resolver.
    #[tokio::test]
    async fn a_failure_names_the_files_the_settings_and_the_way_out() {
        let config = config_with(None, None, None, &[]);
        let error = resolve_configured(&config, "api.internal", 443)
            .await
            .unwrap_err();
        let text = error.to_string();
        for expected in [
            "cannot resolve \"api.internal\"",
            "/etc/resolv.conf",
            "/etc/hosts (missing)",
            DNS_SERVER_ENV,
            RESOLV_CONF_ENV,
            "IP address instead of a host name",
            "NSS",
        ] {
            assert!(text.contains(expected), "{expected} missing from: {text}");
        }
    }

    #[tokio::test]
    async fn a_nameserver_that_does_not_answer_is_named_in_the_failure() {
        // Port 9 on loopback discards nothing and answers nothing; the 1s
        // timeout comes from the resolv.conf written for this test.
        let config = config_with(
            Some("/tmp/does-not-exist/resolv.conf"),
            None,
            None,
            &[(
                "/tmp/does-not-exist/resolv.conf",
                "nameserver 127.0.0.1:9\noptions timeout:1 attempts:1\n",
            )],
        );
        let error = resolve_configured(&config, "api.internal", 443)
            .await
            .unwrap_err();
        let text = error.to_string();
        assert!(text.contains("127.0.0.1:9"), "{text}");
        assert!(text.contains("1s"), "{text}");
    }

    /// A request made through the shared client builder reports the resolver's
    /// reason, not reqwest's bare "error sending request".
    #[tokio::test]
    async fn a_request_error_carries_the_resolution_failure() {
        crate::install_tls_provider();
        let client = attach_with(
            reqwest::Client::builder(),
            leaked(config_with(None, None, None, &[])),
        )
        .build()
        .unwrap();
        let error = client
            .get("http://api.internal/")
            .send()
            .await
            .expect_err("an unresolvable host cannot answer");
        let text = describe(&error);
        assert!(text.contains("cannot resolve"), "{text}");
        assert!(text.contains(DNS_SERVER_ENV), "{text}");
    }

    /// Only a TCP endpoint gets a pre-flight: `auto`, a unix socket and a
    /// named pipe are not host names, and looking one up would turn a working
    /// local endpoint into a resolution error.
    #[test]
    fn only_tcp_endpoints_are_pre_flighted() {
        assert_eq!(tcp_host_port("auto"), None);
        assert_eq!(tcp_host_port("AUTO"), None);
        assert_eq!(tcp_host_port("/run/user/501/future/agent.sock"), None);
        assert_eq!(
            tcp_host_port("unix:///run/user/501/future/agent.sock"),
            None
        );
        assert_eq!(
            tcp_host_port(r"\\\\.\\pipe\\future-agent"),
            None,
            "a Windows named pipe is a path, not a host"
        );
        assert_eq!(
            tcp_host_port("example.com:50051"),
            Some(("example.com".to_string(), 50051))
        );
        assert_eq!(
            tcp_host_port("http://example.com:80"),
            Some(("example.com".to_string(), 80))
        );
        assert_eq!(
            tcp_host_port("https://example.com"),
            Some(("example.com".to_string(), 443))
        );
        assert_eq!(
            tcp_host_port("[2001:db8::1]:8080"),
            Some(("2001:db8::1".to_string(), 8080))
        );
    }

    /// The default path attaches nothing: no resolver on the client builder.
    /// This is the invariant that keeps existing users unaffected, and it is
    /// asserted on the plan rather than on a live lookup because a live lookup
    /// on a healthy machine would pass either way.
    #[test]
    fn the_system_plan_attaches_no_resolver() {
        static SYSTEM: Plan = Plan::System;
        assert!(resolver_for(&SYSTEM).is_none());
        assert!(resolver_for(leaked(config_with(None, Some("192.0.2.1"), None, &[]))).is_some());
    }

    /// On Windows there is no `/etc/resolv.conf` to find, and that absence must
    /// not be read as "this process has no resolver": Windows resolves through
    /// its own DNS client. Without an explicit setting the plan stays system, or
    /// every HTTP client on Windows would lose its resolver.
    #[cfg(windows)]
    #[test]
    fn windows_without_a_setting_keeps_the_system_resolver() {
        assert_eq!(*plan(), Plan::System);
    }
}
