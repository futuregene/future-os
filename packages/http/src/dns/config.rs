//! Where the resolver's configuration comes from — and when it is not ours.
//!
//! [`Plan::detect`] is a pure function over the process environment and a
//! file-existence probe, so the decision that matters most ("is this the
//! system's job or ours?") is testable without touching the real `/etc` or
//! racing other tests through `std::env::set_var`.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The resolv.conf search path, in order, before `$PREFIX` is considered.
///
/// `/etc/resolv.conf` is first and is not just a candidate: when it exists the
/// plan is [`Plan::System`], because that file is exactly what musl reads too.
/// The two others cover hosts whose DNS configuration lives under `/run`
/// (systemd-resolved's own file, and openresolv's) and where `/etc/resolv.conf`
/// is normally a symlink into them — a symlink a minimal image may have
/// dropped while keeping the real file.
const RESOLV_CONF_CANDIDATES: &[&str] = &[
    "/etc/resolv.conf",
    "/run/systemd/resolve/resolv.conf",
    "/run/resolvconf/resolv.conf",
];

/// Where host overrides are read from, before `$PREFIX` is considered.
const HOSTS_CANDIDATES: &[&str] = &["/etc/hosts"];

/// A nameserver the caller names directly; it replaces every file.
pub(crate) const DNS_SERVER_ENV: &str = "FUTURE_DNS_SERVER";
/// A resolv.conf to read instead of the search path.
pub(crate) const RESOLV_CONF_ENV: &str = "FUTURE_RESOLV_CONF";
/// Termux's filesystem root: `$PREFIX/etc/{resolv.conf,hosts}`.
const PREFIX_ENV: &str = "PREFIX";

/// glibc's own defaults for `options`.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_ATTEMPTS: u32 = 2;
/// `ndots` below this many dots sends the name through the search list first.
const DEFAULT_NDOTS: usize = 1;

/// What a lookup should do in this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Hand every lookup to the system resolver — i.e. `getaddrinfo`, which is
    /// what this CLI did before this module existed. Nothing is installed and
    /// nothing changes, down to the error text.
    System,
    /// Resolve here: the system's own configuration is not where libc looks,
    /// or the caller named one explicitly.
    Custom(Box<Config>),
}

/// A resolution configuration read from one file or named on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Config {
    nameservers: Vec<SocketAddr>,
    hosts_files: Vec<PathBuf>,
    search: Vec<String>,
    ndots: usize,
    timeout: Duration,
    attempts: u32,
    /// One line per input, quoted verbatim in a failure so the user can see
    /// what was actually consulted *in this process*.
    sources: Vec<String>,
    /// Set when the configuration that was asked for could not be used; the
    /// resolver then fails with this instead of pretending the files it did
    /// not read were fine.
    problem: Option<String>,
}

/// The inputs [`Plan::detect`] reads, with the environment and the filesystem
/// behind them made explicit.
pub(crate) struct Inputs<'a> {
    pub dns_server: Option<&'a str>,
    pub resolv_conf: Option<&'a str>,
    pub prefix: Option<&'a str>,
    pub exists: &'a dyn Fn(&Path) -> bool,
    pub read: &'a dyn Fn(&Path) -> io::Result<String>,
}

impl Inputs<'_> {
    fn resolv_conf_candidates(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = RESOLV_CONF_CANDIDATES.iter().map(PathBuf::from).collect();
        if let Some(prefix) = self.prefix {
            paths.push(Path::new(prefix).join("etc/resolv.conf"));
        }
        paths
    }

    fn hosts_candidates(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = HOSTS_CANDIDATES.iter().map(PathBuf::from).collect();
        if let Some(prefix) = self.prefix {
            paths.push(Path::new(prefix).join("etc/hosts"));
        }
        paths
    }
}

impl Plan {
    /// Read the environment and decide.
    pub(crate) fn detect() -> Self {
        let dns_server = non_empty(std::env::var(DNS_SERVER_ENV).ok());
        let resolv_conf = non_empty(std::env::var(RESOLV_CONF_ENV).ok());
        let prefix = non_empty(std::env::var(PREFIX_ENV).ok());
        // Windows' `getaddrinfo` is the Windows DNS client, not a reader of
        // `/etc/resolv.conf`, so the absence of that file there says nothing
        // about whether this process can resolve a name. Only an explicit
        // setting moves Windows onto the custom path.
        if cfg!(windows) && dns_server.is_none() && resolv_conf.is_none() {
            return Plan::System;
        }
        let inputs = Inputs {
            dns_server: dns_server.as_deref(),
            resolv_conf: resolv_conf.as_deref(),
            prefix: prefix.as_deref(),
            exists: &|path| path.exists(),
            read: &|path: &Path| std::fs::read_to_string(path),
        };
        Self::detect_with(&inputs)
    }

    pub(crate) fn detect_with(inputs: &Inputs<'_>) -> Self {
        let hosts_files = inputs.hosts_candidates();

        // An explicitly named nameserver wins over every file: it is the one
        // answer a user can give on a machine with no resolv.conf at all, and
        // silently also reading a file would make it non-exclusive.
        if let Some(server) = inputs.dns_server {
            let mut config = Config::empty(hosts_files);
            match parse_dns_server(server) {
                Ok(address) => {
                    config.nameservers = vec![address];
                    config.sources.push(format!(
                        "{DNS_SERVER_ENV}={server}: sole nameserver, no resolv.conf read"
                    ));
                }
                Err(problem) => {
                    config.sources.push(format!("{DNS_SERVER_ENV}={server}"));
                    config.problem = Some(problem);
                }
            }
            config
                .sources
                .push(hosts_source(&config.hosts_files, inputs));
            return Self::Custom(Box::new(config));
        }

        let named = inputs.resolv_conf.map(PathBuf::from);
        let chosen = match &named {
            Some(path) => (inputs.exists)(path).then(|| path.clone()),
            None => inputs
                .resolv_conf_candidates()
                .into_iter()
                .find(|path| (inputs.exists)(path)),
        };

        // The first candidate *is* what libc reads: nothing to add, and the
        // one case where this module must not intervene at all.
        if named.is_none() && chosen.as_deref() == Some(Path::new(RESOLV_CONF_CANDIDATES[0])) {
            return Self::System;
        }

        let mut config = Config::empty(hosts_files);
        match chosen {
            Some(path) => match (inputs.read)(&path) {
                Ok(text) => {
                    let parsed = parse_resolv_conf(&text);
                    config.nameservers = parsed.nameservers;
                    config.search = parsed.search;
                    config.ndots = parsed.ndots;
                    config.timeout = parsed.timeout;
                    config.attempts = parsed.attempts;
                    let how = match &named {
                        Some(_) => RESOLV_CONF_ENV.to_string(),
                        None => "found".to_string(),
                    };
                    config.sources.push(format!(
                        "{how}: {} ({} nameserver(s))",
                        path.display(),
                        config.nameservers.len()
                    ));
                }
                Err(error) => {
                    config
                        .sources
                        .push(format!("{}: unreadable ({error})", path.display()));
                    config.problem = Some(format!("cannot read {}: {error}", path.display()));
                }
            },
            None => {
                let checked = inputs
                    .resolv_conf_candidates()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                match &named {
                    Some(path) => {
                        config
                            .sources
                            .push(format!("{RESOLV_CONF_ENV}: {} (missing)", path.display()));
                        config.problem = Some(format!(
                            "the resolv.conf named by {RESOLV_CONF_ENV} does not exist"
                        ));
                    }
                    None => {
                        config
                            .sources
                            .push(format!("no resolv.conf found; checked {checked}"));
                        config.problem = Some(
                            "no resolv.conf is readable, so this process has no nameserver"
                                .to_string(),
                        );
                    }
                }
            }
        }
        config
            .sources
            .push(hosts_source(&config.hosts_files, inputs));
        Self::Custom(Box::new(config))
    }
}

impl Config {
    fn empty(hosts_files: Vec<PathBuf>) -> Self {
        Self {
            nameservers: Vec::new(),
            hosts_files,
            search: Vec::new(),
            ndots: DEFAULT_NDOTS,
            timeout: DEFAULT_TIMEOUT,
            attempts: DEFAULT_ATTEMPTS,
            sources: Vec::new(),
            problem: None,
        }
    }

    /// The one-line account of this configuration carried into every failure.
    pub(crate) fn describe(&self) -> String {
        self.sources.join("; ")
    }

    /// The file-derived failure, before the host name is known.
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub(crate) fn nameservers(&self) -> &[SocketAddr] {
        &self.nameservers
    }

    pub(crate) fn hosts_files(&self) -> &[PathBuf] {
        &self.hosts_files
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    pub(crate) fn attempts(&self) -> u32 {
        self.attempts
    }

    /// The names to try, in order, for `host`.
    ///
    /// This is glibc's `ndots` rule: a name with at least `ndots` dots is tried
    /// as written before the search list, a shorter one after it. A trailing
    /// dot makes the name absolute and skips the list entirely.
    pub(crate) fn search_candidates(&self, host: &str) -> Vec<String> {
        let trimmed = host.trim_end_matches('.');
        let absolute = trimmed.len() != host.len();
        if absolute || self.search.is_empty() {
            return vec![trimmed.to_string()];
        }
        let mut candidates = Vec::with_capacity(self.search.len() + 1);
        let append_search = || {
            self.search
                .iter()
                .map(|suffix| format!("{trimmed}.{}", suffix.trim_end_matches('.')))
                .collect::<Vec<_>>()
        };
        if trimmed.matches('.').count() >= self.ndots {
            candidates.push(trimmed.to_string());
            candidates.extend(append_search());
        } else {
            candidates.extend(append_search());
            candidates.push(trimmed.to_string());
        }
        candidates
    }
}

fn hosts_source(hosts_files: &[PathBuf], inputs: &Inputs<'_>) -> String {
    let described = hosts_files
        .iter()
        .map(|path| {
            format!(
                "{} ({})",
                path.display(),
                if (inputs.exists)(path) {
                    "present"
                } else {
                    "missing"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("hosts: {described}")
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `1.1.1.1`, `1.1.1.1:53`, `[::1]:53` or `::1`.
fn parse_dns_server(value: &str) -> Result<SocketAddr, String> {
    let value = value.trim();
    if let Ok(address) = value.parse::<SocketAddr>() {
        return Ok(address);
    }
    match value.parse::<IpAddr>() {
        Ok(address) => Ok(SocketAddr::new(address, 53)),
        Err(_) => Err(format!(
            "{DNS_SERVER_ENV}={value} is not an IP address (expected 1.1.1.1 or 1.1.1.1:53)"
        )),
    }
}

/// The subset of resolv.conf this resolver honours.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ResolvConf {
    pub nameservers: Vec<SocketAddr>,
    pub search: Vec<String>,
    pub ndots: usize,
    pub timeout: Duration,
    pub attempts: u32,
}

/// Parse the directives that change what a lookup does.
///
/// Anything else (`sortlist`, `sortlist`, `rotate`, EDNS knobs) is ignored:
/// this resolver sends one A and one AAAA query per name and has nothing to
/// rotate.
pub(crate) fn parse_resolv_conf(text: &str) -> ResolvConf {
    let mut parsed = ResolvConf {
        ndots: DEFAULT_NDOTS,
        timeout: DEFAULT_TIMEOUT,
        attempts: DEFAULT_ATTEMPTS,
        ..ResolvConf::default()
    };
    let mut have_nameservers = false;
    for line in text.lines() {
        let line = strip_comment(line);
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("nameserver") => {
                if let Some(value) = fields.next() {
                    if let Ok(address) = parse_dns_server(value) {
                        parsed.nameservers.push(address);
                        have_nameservers = true;
                    }
                }
            }
            // `domain` is the single-suffix spelling of `search`.
            Some("search" | "domain") => {
                parsed.search = fields.map(|field| field.to_string()).collect();
            }
            Some("options") => {
                for option in fields {
                    apply_option(&mut parsed, option);
                }
            }
            _ => {}
        }
    }
    if !have_nameservers {
        parsed.nameservers.clear();
    }
    parsed
}

fn apply_option(parsed: &mut ResolvConf, option: &str) {
    let (key, value) = match option.split_once(':') {
        Some((key, value)) => (key, Some(value)),
        None => (option, None),
    };
    match (key, value) {
        ("ndots", Some(value)) => {
            if let Ok(ndots) = value.parse::<usize>() {
                parsed.ndots = ndots.min(15);
            }
        }
        ("timeout", Some(value)) => {
            if let Ok(seconds) = value.parse::<u64>() {
                // glibc clamps to 30s; a shorter cap here keeps one unreachable
                // server from stalling a CLI command for minutes.
                parsed.timeout = Duration::from_secs(seconds.clamp(1, 30));
            }
        }
        ("attempts", Some(value)) => {
            if let Ok(attempts) = value.parse::<u32>() {
                parsed.attempts = attempts.clamp(1, 5);
            }
        }
        _ => {}
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find(['#', ';']) {
        Some(index) => &line[..index],
        None => line,
    }
}

/// The addresses `host` is given by `path`, if any.
///
/// `None` means the file could not be read — a missing hosts file is not an
/// error anywhere in the resolver chain, and neither is an unreadable one.
pub(crate) fn hosts_lookup(path: &Path, host: &str) -> Option<Vec<IpAddr>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter_map(|line| {
                let line = strip_comment(line);
                let mut fields = line.split_whitespace();
                let address: IpAddr = fields.next()?.parse().ok()?;
                fields
                    .any(|name| name.eq_ignore_ascii_case(host))
                    .then_some(address)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A fixed environment plus an in-memory filesystem, so the detection
    /// tests decide by the rules and not by this machine's `/etc`.
    struct Fake {
        dns_server: Option<String>,
        resolv_conf: Option<String>,
        prefix: Option<String>,
        files: HashMap<PathBuf, String>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                dns_server: None,
                resolv_conf: None,
                prefix: None,
                files: HashMap::new(),
            }
        }

        fn with(mut self, path: &str, contents: &str) -> Self {
            self.files.insert(PathBuf::from(path), contents.to_string());
            self
        }

        fn detect(&self) -> Plan {
            let exists = |path: &Path| self.files.contains_key(path);
            let read = |path: &Path| {
                self.files
                    .get(path)
                    .cloned()
                    .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
            };
            let inputs = Inputs {
                dns_server: self.dns_server.as_deref(),
                resolv_conf: self.resolv_conf.as_deref(),
                prefix: self.prefix.as_deref(),
                exists: &exists,
                read: &read,
            };
            Plan::detect_with(&inputs)
        }
    }

    fn custom(plan: &Plan) -> &Config {
        match plan {
            Plan::Custom(config) => config,
            Plan::System => panic!("expected a custom plan, got System"),
        }
    }

    /// The one invariant that keeps this feature from changing anything for
    /// existing users: a normal Linux with `/etc/resolv.conf` is not touched.
    #[test]
    fn an_ordinary_linux_keeps_the_system_resolver() {
        let plan = Fake::new()
            .with("/etc/resolv.conf", "nameserver 192.0.2.1\n")
            .with("/etc/hosts", "127.0.0.1 localhost\n")
            .detect();
        assert_eq!(plan, Plan::System);
    }

    #[test]
    fn the_termux_etc_resolv_conf_is_not_a_reason_to_intervene() {
        // Termux has both files; the system resolver reads them, and the plan
        // must stay out of the way.
        let mut plan = Fake::new()
            .with(
                "/data/data/com.termux/files/usr/etc/resolv.conf",
                "nameserver 8.8.8.8\n",
            )
            .with("/etc/resolv.conf", "nameserver 8.8.4.4\n");
        plan.prefix = Some("/data/data/com.termux/files/usr".to_string());
        assert_eq!(plan.detect(), Plan::System);
    }

    #[test]
    fn a_missing_resolv_conf_falls_through_the_candidates_in_order() {
        let plan = Fake::new()
            .with("/run/resolvconf/resolv.conf", "nameserver 203.0.113.9\n")
            .with(
                "/run/systemd/resolve/resolv.conf",
                "nameserver 203.0.113.7\n",
            );
        let detected = plan.detect();
        let config = custom(&detected);
        assert_eq!(
            config.nameservers(),
            &["203.0.113.7:53".parse::<SocketAddr>().unwrap()]
        );
        assert!(config
            .describe()
            .contains("/run/systemd/resolve/resolv.conf"));
    }

    #[test]
    fn nothing_to_read_is_reported_rather_than_silently_failing_later() {
        let fake = Fake::new();
        let detected = fake.detect();
        let config = custom(&detected);
        assert!(config.nameservers().is_empty());
        let problem = config
            .problem()
            .expect("a missing resolv.conf is a problem");
        assert!(problem.contains("no nameserver"), "{problem}");
        // The candidate list belongs in the description, which the failure
        // quotes as "Resolution in this process: …" — one place, not two.
        let described = config.describe();
        assert!(described.contains("/etc/resolv.conf"), "{described}");
        assert!(
            described.contains("/run/systemd/resolve/resolv.conf"),
            "{described}"
        );
    }

    #[test]
    fn the_termux_prefix_is_a_candidate_only_when_nothing_else_exists() {
        let mut fake = Fake::new();
        fake.prefix = Some("/data/data/com.termux/files/usr".to_string());
        fake = fake.with(
            "/data/data/com.termux/files/usr/etc/resolv.conf",
            "nameserver 9.9.9.9\n",
        );
        let detected = fake.detect();
        let config = custom(&detected);
        assert_eq!(
            config.nameservers(),
            &["9.9.9.9:53".parse::<SocketAddr>().unwrap()]
        );
    }

    #[test]
    fn an_explicit_nameserver_replaces_every_file() {
        let mut fake = Fake::new().with("/etc/resolv.conf", "nameserver 192.0.2.1\n");
        fake.dns_server = Some("198.51.100.4".to_string());
        let detected = fake.detect();
        let config = custom(&detected);
        assert_eq!(
            config.nameservers(),
            &["198.51.100.4:53".parse::<SocketAddr>().unwrap()]
        );
        assert!(config.describe().contains("FUTURE_DNS_SERVER=198.51.100.4"));
        assert!(!config.describe().contains("192.0.2.1"));
    }

    #[test]
    fn an_explicit_nameserver_keeps_the_port_it_names() {
        let mut fake = Fake::new();
        fake.dns_server = Some("198.51.100.4:5353".to_string());
        let detected = fake.detect();
        let config = custom(&detected);
        assert_eq!(
            config.nameservers(),
            &["198.51.100.4:5353".parse::<SocketAddr>().unwrap()]
        );
    }

    #[test]
    fn a_malformed_explicit_nameserver_is_a_named_problem() {
        let mut fake = Fake::new();
        fake.dns_server = Some("dns.example.com".to_string());
        let config_plan = fake.detect();
        let config = custom(&config_plan);
        let problem = config.problem().expect("a hostname is not a nameserver");
        assert!(problem.contains("is not an IP address"), "{problem}");
    }

    #[test]
    fn an_explicit_resolv_conf_is_read_even_when_etc_has_one() {
        let mut fake = Fake::new().with("/etc/resolv.conf", "nameserver 192.0.2.1\n");
        fake.resolv_conf = Some("/tmp/elsewhere/resolv.conf".to_string());
        fake = fake.with("/tmp/elsewhere/resolv.conf", "nameserver 203.0.113.5\n");
        let detected = fake.detect();
        let config = custom(&detected);
        assert_eq!(
            config.nameservers(),
            &["203.0.113.5:53".parse::<SocketAddr>().unwrap()]
        );
    }

    #[test]
    fn an_explicit_resolv_conf_that_is_missing_is_reported_by_name() {
        let mut fake = Fake::new().with("/etc/resolv.conf", "nameserver 192.0.2.1\n");
        fake.resolv_conf = Some("/nope/resolv.conf".to_string());
        let plan = fake.detect();
        let config = custom(&plan);
        assert!(config.nameservers().is_empty());
        assert!(config
            .problem()
            .unwrap()
            .contains("named by FUTURE_RESOLV_CONF does not exist"));
    }

    #[test]
    fn resolv_conf_directives_that_change_a_lookup_are_honoured() {
        let parsed = parse_resolv_conf(
            "# comment\n; also a comment\nnameserver 10.0.0.1\nnameserver 2001:db8::1\n\
             search corp.example.com example.com\noptions ndots:3 timeout:2 attempts:4\n",
        );
        assert_eq!(
            parsed.nameservers,
            vec![
                "10.0.0.1:53".parse::<SocketAddr>().unwrap(),
                "[2001:db8::1]:53".parse::<SocketAddr>().unwrap(),
            ]
        );
        assert_eq!(parsed.search, vec!["corp.example.com", "example.com"]);
        assert_eq!(parsed.ndots, 3);
        assert_eq!(parsed.timeout, Duration::from_secs(2));
        assert_eq!(parsed.attempts, 4);
    }

    #[test]
    fn option_values_outside_the_supported_range_are_clamped() {
        let parsed = parse_resolv_conf("options ndots:99 timeout:0 attempts:99\n");
        assert_eq!(parsed.ndots, 15);
        assert_eq!(parsed.timeout, Duration::from_secs(1));
        assert_eq!(parsed.attempts, 5);
    }

    #[test]
    fn the_search_list_follows_the_ndots_rule() {
        let config = Config {
            search: vec!["corp.example.com".to_string()],
            ..Config::empty(Vec::new())
        };
        assert_eq!(
            config.search_candidates("api"),
            vec!["api.corp.example.com".to_string(), "api".to_string()]
        );
        assert_eq!(
            config.search_candidates("api.example.org"),
            vec![
                "api.example.org".to_string(),
                "api.example.org.corp.example.com".to_string()
            ]
        );
        assert_eq!(
            config.search_candidates("api.example.org."),
            vec!["api.example.org".to_string()]
        );
    }

    #[test]
    fn hosts_entries_match_any_alias_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        std::fs::write(
            &path,
            "127.0.0.1 localhost\n# 10.0.0.1 hidden\n10.1.2.3 build localhost.localdomain\n",
        )
        .unwrap();
        assert_eq!(
            hosts_lookup(&path, "LOCALHOST"),
            Some(vec!["127.0.0.1".parse().unwrap()])
        );
        assert_eq!(
            hosts_lookup(&path, "localhost.localdomain"),
            Some(vec!["10.1.2.3".parse().unwrap()])
        );
        assert_eq!(hosts_lookup(&path, "hidden"), Some(Vec::new()));
        assert_eq!(hosts_lookup(&dir.path().join("absent"), "localhost"), None);
    }
}
