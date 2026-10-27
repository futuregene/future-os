//! Per-user local transport discovery for the Future Agent.
//!
//! Native local IPC is the default security boundary: Unix-domain sockets are
//! created below a mode-0700 directory and accept only the current UID;
//! Windows named pipes carry a protected DACL for the current user. TCP is
//! available only when a caller explicitly supplies an address.

use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use crate::proto::future_agent_client::FutureAgentClient;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_stream::Stream;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

/// Sentinel used by CLI/config surfaces that want automatic local discovery.
pub const AUTO_ENDPOINT: &str = "auto";

/// The future a [`TcpNameResolver`] returns: every address the name has.
pub type ResolveFuture = Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>>;

/// A hostname resolver an entry point supplies for TCP Agent endpoints.
///
/// The default — `None` on every public entry point — is `getaddrinfo` through
/// `tokio::net::TcpStream::connect((host, port))`, so a caller that passes
/// nothing gets exactly the resolution it got before this type existed.
///
/// It exists because a binary linked against musl carries a resolver that reads
/// two hard-coded paths (`/etc/hosts`, `/etc/resolv.conf`) with no environment
/// override, so on a system without those files every hostname lookup fails and
/// no amount of configuration in the *rest* of the process can help. The CLI
/// supplies its own resolver there; it must not be installed globally, because
/// a process-global hook would also change how the desktop, the TUI and the
/// host resolve names.
pub type TcpNameResolver = Arc<dyn Fn(String, u16) -> ResolveFuture + Send + Sync>;

/// gRPC message-size cap, shared by the Agent server and every client.
///
/// tonic's default *decoding* limit is 4 MiB, which one `get_messages`
/// response (a whole session transcript) routinely exceeds: the server encodes
/// it happily and the client then fails the call with `decoded message length
/// too large: found N bytes, the limit is: 4194304 bytes`. Client *encoding*
/// and server *decoding* are capped at the same value so a large request can
/// never be sent either. Build clients through [`agent_client`] to inherit it.
pub const MAX_GRPC_MESSAGE_SIZE: usize = 32 * 1024 * 1024;

/// A typed Agent client over `channel`, carrying the shared message-size cap.
///
/// Nothing else reapplies [`MAX_GRPC_MESSAGE_SIZE`] — the generated client's
/// builder keeps tonic's 4 MiB decoding default — so constructing
/// `FutureAgentClient` directly silently reintroduces the too-large-response
/// failure in whichever call happens to return a large session payload.
pub fn agent_client(channel: Channel) -> FutureAgentClient<Channel> {
    FutureAgentClient::new(channel)
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
        .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE)
}

/// A concrete transport that clients may try.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEndpoint {
    Local,
    Tcp(String),
}

impl AgentEndpoint {
    pub fn label(&self) -> String {
        match self {
            Self::Local => local_endpoint_label(),
            Self::Tcp(addr) => normalize_tcp_uri(addr),
        }
    }
}

/// The endpoint and connected HTTP/2 channel selected by discovery.
pub struct ConnectedChannel {
    pub endpoint: AgentEndpoint,
    pub channel: Channel,
}

#[derive(Debug, Clone)]
pub struct ConnectError {
    details: String,
    pub kind: Option<io::ErrorKind>,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.details)
    }
}

impl std::error::Error for ConnectError {}

/// Build the connection plan. `None`, an empty value, and `auto` select the
/// per-user local transport. An explicit TCP endpoint is authoritative: a
/// failed remote/development target must not silently redirect commands to an
/// unrelated local Agent.
pub fn connection_plan(configured: Option<&str>) -> Vec<AgentEndpoint> {
    let configured = configured
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(AUTO_ENDPOINT));
    match configured {
        Some(addr) => vec![AgentEndpoint::Tcp(addr.to_string())],
        None => vec![AgentEndpoint::Local],
    }
}

/// Connect to the first reachable endpoint in the discovery plan.
///
/// `connect_timeout` only bounds endpoint establishment. `request_timeout`
/// optionally installs a channel-wide deadline for clients whose entire RPC
/// surface is intentionally bounded; long-lived clients should pass `None`
/// and set deadlines on individual requests where appropriate.
pub async fn connect_channel(
    configured: Option<&str>,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
) -> Result<ConnectedChannel, ConnectError> {
    connect_channel_with_policy(configured, connect_timeout, request_timeout, false, None).await
}

/// A verified business channel must never transparently reconnect to a
/// replacement (including legacy servers that ignore instance headers). A new
/// physical channel requires a new readiness handshake.
pub async fn connect_pinned_channel(
    configured: Option<&str>,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
) -> Result<ConnectedChannel, ConnectError> {
    connect_pinned_channel_with(configured, connect_timeout, request_timeout, None).await
}

/// [`connect_pinned_channel`] with an explicit hostname resolver for TCP
/// endpoints. `None` is the default and resolves through `getaddrinfo`.
pub async fn connect_pinned_channel_with(
    configured: Option<&str>,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
    resolver: Option<TcpNameResolver>,
) -> Result<ConnectedChannel, ConnectError> {
    connect_channel_with_policy(configured, connect_timeout, request_timeout, true, resolver).await
}

async fn connect_channel_with_policy(
    configured: Option<&str>,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
    pinned: bool,
    resolver: Option<TcpNameResolver>,
) -> Result<ConnectedChannel, ConnectError> {
    connect_channel_with_connector(configured, connect_timeout, |endpoint| {
        let resolver = resolver.clone();
        async move {
            connect_one(
                &endpoint,
                connect_timeout,
                request_timeout,
                pinned,
                resolver,
            )
            .await
        }
    })
    .await
}

async fn connect_channel_with_connector<F, Fut>(
    configured: Option<&str>,
    connect_timeout: Duration,
    mut connect: F,
) -> Result<ConnectedChannel, ConnectError>
where
    F: FnMut(AgentEndpoint) -> Fut,
    Fut: std::future::Future<Output = Result<Channel, Box<dyn std::error::Error + Send + Sync>>>,
{
    let mut failures = Vec::new();
    let mut kind = None;
    for endpoint in connection_plan(configured) {
        let label = endpoint.label();
        let attempt = tokio::time::timeout(connect_timeout, connect(endpoint.clone())).await;
        match attempt {
            Ok(Ok(channel)) => return Ok(ConnectedChannel { endpoint, channel }),
            Ok(Err(error)) => {
                kind = io_error_kind(error.as_ref());
                failures.push(format!("{label}: {error}"));
            }
            Err(_) => {
                kind = Some(io::ErrorKind::TimedOut);
                failures.push(format!("{label}: connection timed out"));
            }
        }
    }
    Err(ConnectError {
        kind,
        details: format!(
            "unable to connect to Future Agent ({})",
            failures.join("; ")
        ),
    })
}

fn io_error_kind(mut error: &(dyn std::error::Error + 'static)) -> Option<io::ErrorKind> {
    loop {
        if let Some(error) = error.downcast_ref::<io::Error>() {
            return Some(error.kind());
        }
        error = error.source()?;
    }
}

async fn connect_one(
    endpoint: &AgentEndpoint,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
    pinned: bool,
    resolver: Option<TcpNameResolver>,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    match endpoint {
        AgentEndpoint::Tcp(addr) => {
            let endpoint =
                Endpoint::from_shared(normalize_tcp_uri(addr))?.connect_timeout(connect_timeout);
            let endpoint = match request_timeout {
                Some(timeout) => endpoint.timeout(timeout),
                None => endpoint,
            };
            if !pinned {
                return Ok(endpoint.connect().await?);
            }
            if endpoint.uri().scheme_str() == Some("https") {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "HTTPS Agent endpoints require an explicitly configured TLS connector; refusing plaintext transport").into());
            }
            let host = endpoint
                .uri()
                .host()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "TCP endpoint has no host")
                })?
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned();
            let port = endpoint.uri().port_u16().unwrap_or(
                if endpoint.uri().scheme_str() == Some("https") {
                    443
                } else {
                    80
                },
            );
            let used = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            Ok(endpoint
                .connect_with_connector(service_fn(move |_| {
                    let used = used.clone();
                    let host = host.clone();
                    let resolver = resolver.clone();
                    async move {
                        claim_connection(&used, true)?;
                        let stream = match resolver {
                            None => tokio::net::TcpStream::connect((host.as_str(), port)).await?,
                            Some(resolver) => connect_resolved(resolver, host, port).await?,
                        };
                        stream.set_nodelay(true)?;
                        Ok::<_, io::Error>(TokioIo::new(stream))
                    }
                }))
                .await?)
        }
        AgentEndpoint::Local => connect_local_with_policy(request_timeout, pinned).await,
    }
}

/// Connect to the first address the supplied resolver returns for `host`.
///
/// Every address is tried in the order given and the last failure is reported,
/// which is what `TcpStream::connect((host, port))` does with the address list
/// `getaddrinfo` hands it — the resolver replaces where the list comes from,
/// not how it is used.
async fn connect_resolved(
    resolver: TcpNameResolver,
    host: String,
    port: u16,
) -> io::Result<tokio::net::TcpStream> {
    let addresses = resolver(host.clone(), port).await?;
    let mut last_error = None;
    for address in addresses {
        match tokio::net::TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("no address for {host}"))
    }))
}

fn claim_connection(used: &std::sync::atomic::AtomicBool, pinned: bool) -> io::Result<()> {
    if pinned && used.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Agent connection changed; a fresh readiness handshake is required",
        ));
    }
    Ok(())
}

fn normalize_tcp_uri(addr: &str) -> String {
    if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        format!("http://{addr}")
    }
}

#[cfg(unix)]
pub fn local_socket_path() -> std::path::PathBuf {
    local_socket_path_from(
        std::env::var_os("FUTURE_AGENT_SOCKET"),
        crate::home::future_home_override(),
        std::env::var_os("HOME"),
    )
}

/// Local endpoint of the instance that owns `future_home` (see
/// [`crate::home`]), with every environment input injected for testability.
///
/// The endpoint always lives under the instance's FutureOS home, so one
/// directory owns the whole instance boundary (lock, database, sessions and
/// IPC endpoint together). A shared runtime directory such as
/// `$XDG_RUNTIME_DIR` is deliberately not consulted: it is per-user rather
/// than per-instance, and on hosts without a systemd login session (HPC
/// compute nodes) it commonly points at a directory the user cannot create
/// or write, which used to abort the Agent with `EACCES`.
#[cfg(unix)]
fn local_socket_path_from(
    explicit: Option<std::ffi::OsString>,
    future_home: Option<std::path::PathBuf>,
    home: Option<std::ffi::OsString>,
) -> std::path::PathBuf {
    // An explicit override is the user's decision and is used verbatim: it takes
    // no part in the length fallback below, because silently relocating a path
    // somebody named by hand is worse than letting bind(2) say it is too long.
    if let Some(path) = explicit.filter(|v| !v.is_empty()) {
        return path.into();
    }
    let wanted = match future_home {
        Some(future_home) => future_home.join("run").join("agent.sock"),
        None => home
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".future")
            .join("run")
            .join("agent.sock"),
    };
    if fits_sun_path(&wanted) {
        return wanted;
    }
    short_local_socket_path(&wanted)
}

/// `sun_path` holds 104 bytes on macOS and 108 on Linux, terminator included, so
/// 103 (macOS) is already too long. `std` refuses above that without reaching the
/// kernel: `InvalidInput`, "path must be shorter than SUN_LEN" — *not* `EPERM`,
/// and not `ENAMETOOLONG`. The smaller of the two platforms governs, so one
/// answer covers both.
#[cfg(unix)]
const SUN_PATH_MAX: usize = 103;

/// Whether `path` can be bound as a unix socket.
///
/// The comparison is on the path as *computed*, never canonicalized: every
/// process derives it from the same environment inputs, and resolving symlinks
/// (`/tmp` is one on macOS, `/var` -> `/private/var` is another) could make two
/// processes disagree and end up with two endpoints for one home.
#[cfg(unix)]
fn fits_sun_path(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().len() <= SUN_PATH_MAX
}

/// The short endpoint used instead of one that cannot fit in `sun_path`.
///
/// A `FUTURE_HOME` deep enough to overrun the limit (a git worktree, a nested
/// project directory, a CI checkout) made the Agent fail to bind at all: startup
/// stopped at "binding the gRPC endpoint" with a message that never mentioned the
/// home. Those homes have another property that makes relocating safe: the old
/// path **cannot be bound by the previous release either**, so there is no "old
/// Agent still serving on the long path" to stay compatible with.
///
/// The name is recomputable from `(uid, wanted)`, with no randomness and no
/// timestamp, so two processes that derive it independently get the same string.
///
/// The base is the fixed `/tmp`, deliberately not `$TMPDIR`: on macOS `$TMPDIR`
/// is per-session (launchd and a terminal shell can get different directories),
/// and a GUI-hosted Agent and a terminal client would then compute two different
/// endpoints for one home. `/tmp` is a symlink to `/private/tmp` there, and both
/// sides spell `/tmp/...` rather than resolving it, which is what makes them
/// agree. (Do not switch this to `$TMPDIR` without solving that.)
#[cfg(unix)]
fn short_local_socket_path(wanted: &std::path::Path) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;

    let mut hasher = Sha256::new();
    hasher.update(effective_uid().to_le_bytes());
    hasher.update(wanted.as_os_str().as_bytes());
    let digest = hasher.finalize();
    let stem: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    endpoint_dir().join(format!("{stem}.sock"))
}

/// The owned directory short endpoints live in: `/tmp/future-<uid>`.
///
/// `/tmp` is world-writable, so this path can be pre-created by anybody. A
/// symlink there would redirect our socket somewhere the attacker chooses, and a
/// directory owned by another user would let them read or replace it. Every
/// condition is therefore checked with `lstat` and enforced by refusing, never by
/// "fixing it up": chmod'ing a directory someone else made is exactly the mistake
/// this avoids, and it also fails anyway when the parent (`/tmp` itself) is not
/// ours to chmod.
#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and only reads the process's own
    // credentials; it cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
pub fn endpoint_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("/tmp").join(format!("future-{}", effective_uid()))
}

/// Prepare [`endpoint_dir`] and return it, or say why it is unsafe to use.
///
/// Called on the Agent's bind path (through [`bind_local`]) so a squatted
/// directory is refused before anything is written into it.
#[cfg(unix)]
pub fn ensure_endpoint_dir() -> std::io::Result<std::path::PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let dir = endpoint_dir();
    let uid = effective_uid();
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "refusing to use {}: it is a symbolic link; remove it (or point \
                         FUTURE_AGENT_SOCKET elsewhere) so future can create its own directory",
                        dir.display()
                    ),
                ));
            }
            if !meta.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("refusing to use {}: it is not a directory", dir.display()),
                ));
            }
            if meta.uid() != uid {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "refusing to use {}: it is owned by uid {}, not {uid}",
                        dir.display(),
                        meta.uid()
                    ),
                ));
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o700 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "refusing to use {}: mode is {mode:o}, expected 700; run `chmod 700 {}`",
                        dir.display(),
                        dir.display()
                    ),
                ));
            }
            Ok(dir)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&dir)?;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            // Re-check what actually appeared: create_dir is atomic against a
            // concurrent creator, but the winner's object is what we must trust.
            ensure_endpoint_dir()
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
pub fn local_endpoint_label() -> String {
    format!("unix://{}", local_socket_path().display())
}

#[cfg(windows)]
pub fn local_endpoint_label() -> String {
    format!(
        "npipe://{}",
        local_pipe_name().trim_start_matches(r"\\.\pipe\")
    )
}

#[cfg(all(test, windows))]
async fn connect_local(
    request_timeout: Option<Duration>,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    connect_local_with_policy(request_timeout, false).await
}

#[cfg(unix)]
async fn connect_local_with_policy(
    request_timeout: Option<Duration>,
    pinned: bool,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    let path = local_socket_path();
    let endpoint = Endpoint::try_from("http://future-agent.local")?;
    let endpoint = match request_timeout {
        Some(timeout) => endpoint.timeout(timeout),
        None => endpoint,
    };
    let used = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let channel = endpoint
        .connect_with_connector(service_fn(move |_| {
            let used = used.clone();
            let path = path.clone();
            async move {
                claim_connection(&used, pinned)?;
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(TokioIo::new)
            }
        }))
        .await?;
    Ok(channel)
}

#[cfg(windows)]
async fn connect_local_with_policy(
    request_timeout: Option<Duration>,
    pinned: bool,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name = local_pipe_name();
    let endpoint = Endpoint::try_from("http://future-agent.local")?;
    let endpoint = match request_timeout {
        Some(timeout) => endpoint.timeout(timeout),
        None => endpoint,
    };
    let used = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let channel = endpoint
        .connect_with_connector(service_fn(move |_| {
            let used = used.clone();
            let pipe_name = pipe_name.clone();
            async move {
                claim_connection(&used, pinned)?;
                loop {
                    match ClientOptions::new().open(&pipe_name) {
                        Ok(pipe) => return Ok(TokioIo::new(pipe)),
                        Err(error) if error.raw_os_error() == Some(231) => {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }))
        .await?;
    Ok(channel)
}

/// IO accepted by the local server and consumed by tonic.
pub struct LocalIo {
    #[cfg(unix)]
    inner: tokio::net::UnixStream,
    #[cfg(windows)]
    inner: tokio::net::windows::named_pipe::NamedPipeServer,
}

impl tonic::transport::server::Connected for LocalIo {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for LocalIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for LocalIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub type LocalIncoming = Pin<Box<dyn Stream<Item = Result<LocalIo, io::Error>> + Send + 'static>>;

/// Attach actionable guidance when the socket directory cannot be created or
/// inspected: a FutureOS home on NFS (or any filesystem that does not support
/// Unix-domain sockets) or without write permission is the common cause.
/// An endpoint that cannot fit in `sun_path` at all.
///
/// Reached only when even the short fallback is too long, or when the caller
/// bound an explicit path that is. `std` says "path must be shorter than
/// SUN_LEN", which names neither the setting responsible nor a way out; a home
/// deep enough to trigger this is usually one somebody configured on purpose.
#[cfg(unix)]
fn sun_path_error(path: &std::path::Path) -> io::Error {
    use std::os::unix::ffi::OsStrExt;
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "the Agent's local endpoint is too long for a unix socket ({} bytes, limit {SUN_PATH_MAX}): {}; \
             shorten FUTURE_HOME (and so the default endpoint under <home>/run), or set \
             FUTURE_AGENT_SOCKET to a short path yourself",
            path.as_os_str().as_bytes().len(),
            path.display()
        ),
    )
}

#[cfg(unix)]
fn bind_dir_error(error: io::Error, parent: &std::path::Path) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "cannot prepare local socket directory {}: {error}; if the FutureOS home is on NFS \
             or otherwise unsuitable, point FUTURE_HOME (or FUTURE_AGENT_SOCKET) at a directory \
             on a local filesystem",
            parent.display()
        ),
    )
}

#[cfg(unix)]
mod unix_endpoint;
#[cfg(unix)]
pub use unix_endpoint::LocalEndpointReservation;

/// Reserve the native endpoint while holding the data-root lock, before storage
/// initialization. The reservation remains owned through listener teardown.
#[cfg(unix)]
pub async fn reserve_local_endpoint() -> io::Result<LocalEndpointReservation> {
    LocalEndpointReservation::acquire(local_socket_path()).await
}

#[cfg(unix)]
pub async fn bind_local() -> io::Result<LocalIncoming> {
    bind_local_at(local_socket_path()).await
}

#[cfg(unix)]
async fn bind_local_at(path: std::path::PathBuf) -> io::Result<LocalIncoming> {
    // A short fallback endpoint lives in a world-writable directory, so verify it
    // before anything is created inside; an explicit or in-home path needs no
    // such check (its directory is the instance's own).
    if path.parent() == Some(endpoint_dir().as_path()) {
        ensure_endpoint_dir()?;
    }
    if !fits_sun_path(&path) {
        return Err(sun_path_error(&path));
    }
    LocalEndpointReservation::acquire(path).await?.bind().await
}

#[cfg(windows)]
fn local_pipe_name() -> String {
    local_pipe_name_for(
        current_user_sid_string().unwrap_or_else(|_| "unknown-user".to_string()),
        crate::home::future_home_override(),
    )
}

/// Named pipe of the instance that owns `future_home`.
///
/// The pipe is otherwise keyed by the current user's SID alone, which two
/// isolated instances on the same account would share — the isolated instance
/// therefore gets its own pipe, tagged by its FutureOS home.
#[cfg(windows)]
fn local_pipe_name_for(sid: String, future_home: Option<std::path::PathBuf>) -> String {
    match future_home {
        Some(home) => format!(
            r"\\.\pipe\future-agent-{sid}-{}",
            crate::home::home_tag(&home)
        ),
        None => format!(r"\\.\pipe\future-agent-{sid}"),
    }
}

#[cfg(windows)]
fn current_user_sid_string() -> io::Result<String> {
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut bytes = 0;
        unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_u8; bytes as usize];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let mut sid_text = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        while unsafe { *sid_text.add(len) } != 0 {
            len += 1;
        }
        let value = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_text, len) });
        unsafe { LocalFree(sid_text.cast()) };
        Ok(value)
    })();
    unsafe { CloseHandle(token) };
    result
}

#[cfg(windows)]
fn create_protected_pipe(
    name: &str,
    first: bool,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use std::ffi::c_void;
    use std::ptr;
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let sid = current_user_sid_string()?;
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor: *mut c_void = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    let result = unsafe {
        options.create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    };
    unsafe { LocalFree(descriptor.cast()) };
    result
}

#[cfg(windows)]
pub async fn bind_local() -> io::Result<LocalIncoming> {
    bind_local_at(local_pipe_name()).await
}

/// Bind an explicit pipe name, for callers that must not resolve the name from the
/// shared environment. `bind_local` is the production path; tests use this so a
/// name of their own cannot be turned into the default instance's pipe.
#[cfg(windows)]
async fn bind_local_at(name: String) -> io::Result<LocalIncoming> {
    let first = create_protected_pipe(&name, true)?;
    let incoming = async_stream::stream! {
        let mut server = first;
        loop {
            match server.connect().await {
                Ok(()) => match create_protected_pipe(&name, false) {
                    Ok(next) => {
                        let connected = std::mem::replace(&mut server, next);
                        yield Ok(LocalIo { inner: connected });
                    }
                    Err(error) => {
                        yield Err(error);
                        break;
                    },
                },
                Err(error) => {
                        yield Err(error);
                        break;
                    },
            }
        }
    };
    Ok(Box::pin(incoming))
}

#[cfg(test)]
mod tests {
    use super::*;
    // `StreamExt::next` is driven from the unix-only IPC accept test; importing
    // it unconditionally makes the Windows build fail on an unused import.
    #[cfg(unix)]
    use tokio_stream::StreamExt;

    #[test]
    fn automatic_mode_never_falls_back_to_the_shared_tcp_port() {
        assert_eq!(connection_plan(None), vec![AgentEndpoint::Local]);
        assert_eq!(connection_plan(Some("")), vec![AgentEndpoint::Local]);
        assert_eq!(connection_plan(Some("auto")), vec![AgentEndpoint::Local]);
    }

    #[test]
    fn explicitly_configured_tcp_never_falls_back_to_local_ipc() {
        assert_eq!(
            connection_plan(Some("http://127.0.0.1:50051")),
            vec![AgentEndpoint::Tcp("http://127.0.0.1:50051".into())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_socket_env_wins_over_a_redirected_future_home() {
        let path = local_socket_path_from(
            Some("/tmp/explicit.sock".into()),
            Some(std::path::PathBuf::from("/tmp/futureos-home")),
            Some("/home/user".into()),
        );
        assert_eq!(path, std::path::PathBuf::from("/tmp/explicit.sock"));
    }

    /// A redirected FutureOS home owns its endpoint under `<home>/run` — the
    /// instance boundary and the FutureOS home boundary are the same thing.
    /// Regression guard for multi-instance isolation.
    #[cfg(unix)]
    #[test]
    fn redirected_future_home_owns_its_endpoint() {
        let home = std::path::PathBuf::from("/tmp/futureos-home");
        let path = local_socket_path_from(None, Some(home.clone()), Some("/home/user".into()));
        assert_eq!(path, home.join("run").join("agent.sock"));
    }

    /// Without a redirect the endpoint stays under the default FutureOS home;
    /// `$XDG_RUNTIME_DIR` is deliberately ignored (per-user, not per-instance,
    /// and unwritable on hosts without a systemd login session).
    #[cfg(unix)]
    #[test]
    fn without_a_redirect_the_endpoint_stays_under_the_default_home() {
        assert_eq!(
            local_socket_path_from(None, None, Some("/home/user".into())),
            std::path::PathBuf::from("/home/user/.future/run/agent.sock")
        );
        assert_eq!(
            local_socket_path_from(None, None, None),
            std::env::temp_dir()
                .join(".future")
                .join("run")
                .join("agent.sock")
        );
    }

    /// C's central promise: a home that cannot fit keeps its endpoint inside the
    /// home, byte for byte. Turning the fallback on unconditionally would move
    /// every existing installation's socket.
    #[cfg(unix)]
    #[test]
    fn a_home_that_fits_keeps_the_layout_unchanged() {
        let home = std::path::PathBuf::from("/tmp/short-home");
        assert_eq!(
            local_socket_path_from(None, Some(home.clone()), None),
            home.join("run").join("agent.sock")
        );
        assert_eq!(
            local_socket_path_from(None, None, Some("/home/user".into())),
            std::path::PathBuf::from("/home/user/.future/run/agent.sock")
        );
    }

    /// An explicit override is the user's choice: never relocated, even when it
    /// is too long. Silently moving it would be worse than the error bind gives.
    #[cfg(unix)]
    #[test]
    fn an_explicit_socket_override_is_never_relocated() {
        let long = format!("/tmp/{}/explicit.sock", "d/".repeat(60));
        let path = local_socket_path_from(Some(long.clone().into()), None, None);
        assert_eq!(path, std::path::PathBuf::from(&long));
        assert!(!fits_sun_path(&path), "the fixture must exceed the limit");
    }

    /// A home too deep for `sun_path` gets a short endpoint that (a) fits,
    /// (b) is recomputable — two processes must agree — and (c) is still
    /// per-home, so two instances never share one socket.
    #[cfg(unix)]
    #[test]
    fn a_home_too_deep_gets_a_recomputable_short_endpoint() {
        let deep = std::path::PathBuf::from(format!("/tmp/{}", "d/".repeat(60)));
        let first = local_socket_path_from(None, Some(deep.clone()), None);
        assert!(fits_sun_path(&first), "still too long: {}", first.display());
        assert_eq!(
            first,
            local_socket_path_from(None, Some(deep), None),
            "the short name must not depend on randomness or time"
        );
        assert!(
            first.parent() == Some(endpoint_dir().as_path()),
            "a short endpoint belongs in the owned temp directory: {}",
            first.display()
        );
        let other = std::path::PathBuf::from(format!("/tmp/{}x", "d/".repeat(60)));
        assert_ne!(
            first,
            local_socket_path_from(None, Some(other), None),
            "two homes must not share an endpoint"
        );
        // The lease file sits beside it and is a plain file, so `PATH_MAX`, not
        // `sun_path`, governs it; assert it is far below that and not silently
        // assumed to share the socket's budget.
        let lease = format!("{}.owner.lock", first.display());
        assert!(lease.len() < 200, "lease path unexpectedly long: {lease}");
    }

    /// `/tmp` is world-writable: a pre-created directory there can belong to
    /// anybody, or be a symlink pointing anywhere. Both are refused rather than
    /// repaired — chmod'ing someone else's directory is the attack, not the fix.
    #[cfg(unix)]
    #[test]
    fn a_squatted_endpoint_directory_is_refused() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = endpoint_dir();
        let _ = std::fs::remove_file(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        // 1. A symlink where the directory should be.
        let target = std::env::temp_dir().join("future-squat-target-test");
        std::fs::create_dir_all(&target).unwrap();
        if symlink(&target, &dir).is_ok() {
            let error = ensure_endpoint_dir().expect_err("a symlink must be refused");
            assert!(
                error.to_string().contains("symbolic link"),
                "the refusal must name the cause: {error}"
            );
            std::fs::remove_file(&dir).unwrap();
        }
        let _ = std::fs::remove_dir_all(&target);

        // 2. A directory with the wrong mode.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = ensure_endpoint_dir().expect_err("a 0755 directory must be refused");
        assert!(
            error.to_string().contains("expected 700"),
            "the refusal must say what to fix: {error}"
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        // 3. The legitimate case still works, and creates it when absent.
        let _ = std::fs::remove_dir_all(&dir);
        let made = ensure_endpoint_dir().expect("a fresh owned directory is fine");
        assert_eq!(made, dir);
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the created directory must be private");
    }

    /// The fallback's stated limit: one byte over is enough to relocate.
    #[cfg(unix)]
    #[test]
    fn the_limit_is_where_measured() {
        let prefix = "/tmp/";
        let suffix = "/run/agent.sock";
        let make = |total: usize| {
            let pad = total - prefix.len() - suffix.len();
            std::path::PathBuf::from(format!("{prefix}{}{suffix}", "h".repeat(pad)))
        };
        let at_limit = make(SUN_PATH_MAX);
        assert!(fits_sun_path(&at_limit), "{} must fit", at_limit.display());
        let over = make(SUN_PATH_MAX + 1);
        assert!(!fits_sun_path(&over), "{} must not fit", over.display());
    }

    #[cfg(windows)]
    #[test]
    fn redirected_future_home_gets_its_own_named_pipe() {
        let default = local_pipe_name_for("S-1-5-21".into(), None);
        assert_eq!(default, r"\\.\pipe\future-agent-S-1-5-21");
        let isolated = local_pipe_name_for(
            "S-1-5-21".into(),
            Some(std::path::PathBuf::from(r"C:\futureos-home")),
        );
        assert_ne!(isolated, default);
        assert!(isolated.starts_with(r"\\.\pipe\future-agent-S-1-5-21-"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreign_socket_directory_is_rejected_before_chmod() {
        use std::os::unix::fs::MetadataExt;
        let root = std::path::Path::new("/");
        if std::fs::metadata(root).unwrap().uid() == unsafe { libc::geteuid() } {
            return; // root cannot use this unprivileged fixture
        }
        let result = bind_local_at(root.join("future-ownership-test.sock")).await;
        let error = match result {
            Err(e) => e,
            Ok(_) => panic!("foreign directory accepted"),
        };
        assert!(
            error.to_string().contains("not owned by the current user"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unpolled_local_listener_releases_its_endpoint_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("agent.sock");
        let incoming = bind_local_at(socket.clone()).await.unwrap();
        assert!(socket.exists());
        drop(incoming);
        assert!(!socket.exists(), "cancelled startup must remove its socket");
        let next = bind_local_at(socket.clone()).await.unwrap();
        drop(next);
        assert!(!socket.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_socket_is_private_and_accepts_the_current_user() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("nested/agent.sock");
        let mut incoming = bind_local_at(socket.clone()).await.unwrap();
        assert_eq!(
            std::fs::metadata(socket.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let client = tokio::net::UnixStream::connect(&socket).await.unwrap();
        let accepted = std::pin::Pin::new(&mut incoming).next().await.unwrap();
        assert!(accepted.is_ok());
        drop(client);
    }

    /// The named pipe is keyed by the user's SID, and an instance with its own
    /// FutureOS home gets its own pipe. Two isolated instances on one account must
    /// not share an endpoint - otherwise the second one steals the first's traffic.
    #[cfg(windows)]
    #[test]
    fn a_redirected_future_home_gets_its_own_named_pipe() {
        let default = local_pipe_name_for("S-1-5-21-100".to_string(), None);
        assert_eq!(default, r"\\.\pipe\future-agent-S-1-5-21-100");

        let home = std::path::PathBuf::from(r"C:\tmp\futureos-home");
        let tagged = local_pipe_name_for("S-1-5-21-100".to_string(), Some(home.clone()));
        assert_ne!(
            tagged, default,
            "an instance with its own home must not reuse the default pipe"
        );
        assert_eq!(
            tagged,
            format!(
                r"\\.\pipe\future-agent-S-1-5-21-100-{}",
                crate::home::home_tag(&home)
            ),
            "the tag must be derived from the home, so the name is stable across restarts"
        );
        // A different home yields a different pipe (the tag is not a constant).
        let other = local_pipe_name_for(
            "S-1-5-21-100".to_string(),
            Some(std::path::PathBuf::from(r"C:\tmp\other-home")),
        );
        assert_ne!(tagged, other);
    }

    /// The pipe is created with a DACL that grants only SYSTEM and the current user,
    /// so a local process running as someone else cannot connect. That rests on
    /// `current_user_sid_string` actually resolving a SID - if it silently returned an
    /// empty string, the SDDL would be built around `;;;` and the ACL would be wrong.
    #[cfg(windows)]
    #[test]
    fn the_current_user_sid_is_resolved_to_a_real_sid_string() {
        let sid = current_user_sid_string().expect("the process token must yield a SID");
        assert!(
            sid.starts_with("S-1-"),
            "a SID string starts with S-1-, got {sid:?}"
        );
        assert!(sid.len() > 4, "the SID must not be empty: {sid:?}");
    }

    /// End-to-end over a REAL named pipe, with a name derived from an isolated home
    /// so the test never binds the default instance's endpoint (a running agent owns
    /// that one, and `first_pipe_instance` would make this fail or, worse, interfere).
    ///
    /// This is the only thing that executes the Windows accept loop, the protected
    /// pipe creation, and `LocalIo`'s four `poll_*` delegations - the unix test covers
    /// the same shapes on the other platform, but neither platform's test compiles for
    /// the other, so the Windows half cannot be inferred from the unix one.
    #[cfg(windows)]
    #[tokio::test]
    async fn named_pipe_accepts_the_current_user_and_round_trips_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_stream::StreamExt;

        // The pipe name is passed explicitly rather than through `FUTURE_HOME`: the
        // variable is process-global, and another test in this binary setting or
        // restoring it mid-flight would rename this bind onto the default instance's
        // pipe - which a running agent owns, so the bind fails with access denied.
        let home = tempfile::tempdir().unwrap();
        let pipe = local_pipe_name_for(
            current_user_sid_string().expect("the process token must yield a SID"),
            Some(home.path().to_path_buf()),
        );
        assert!(
            pipe.contains(&crate::home::home_tag(home.path())),
            "the test must bind its OWN pipe, not the default instance's: {pipe}"
        );

        let mut incoming = bind_local_at(pipe.clone())
            .await
            .expect("the pipe must bind");

        let mut client = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(&pipe)
            .expect("a client of the current user must be accepted");

        let accepted = std::pin::Pin::new(&mut incoming)
            .next()
            .await
            .expect("the server must accept the connection")
            .expect("the accepted IO must be usable");
        let mut server = accepted;

        // tonic asks for the connect info before it serves the stream; the value is
        // `()` here (the pipe is already scoped to this user), and the arm must still
        // be present or tonic cannot use this IO at all.
        assert_eq!(
            <LocalIo as tonic::transport::server::Connected>::connect_info(&server),
            ()
        );

        // client -> server (poll_read) and server -> client (poll_write),
        // with an explicit flush so `poll_flush` is driven too.
        client.write_all(b"ping from the client").await.unwrap();
        client.flush().await.unwrap();
        let mut buf = [0_u8; 20];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping from the client");

        server.write_all(b"pong").await.unwrap();
        server.flush().await.unwrap();
        let mut reply = [0_u8; 4];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"pong");

        // An orderly shutdown: the server's poll_shutdown path.
        server.shutdown().await.unwrap();
        drop(client);
    }

    /// A configured `request_timeout` must be applied to the endpoint. Both arms of
    /// the `match` exist for the same reason - without the `Some` arm a long request
    /// would hang forever on a pipe that never answers - and the timeout rides on the
    /// channel, so it is set before the connection is attempted (and must therefore be
    /// observable even when the connection fails).
    #[cfg(windows)]
    #[tokio::test]
    async fn a_request_timeout_is_applied_before_the_local_pipe_is_opened() {
        let home = tempfile::tempdir().unwrap();
        let previous = std::env::var_os(crate::home::FUTURE_HOME_ENV);
        std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

        // No server is listening on this run's pipe, so the open fails with
        // ERROR_FILE_NOT_FOUND. That is the NON-retryable arm: only ERROR_PIPE_BUSY
        // (231) is retried, because retrying a missing pipe would spin forever.
        let error = connect_local(Some(Duration::from_secs(5)))
            .await
            .expect_err("a pipe with no server must not connect");
        // Note the detail: tonic reports the connector's io failure as a bare
        // "transport error", so the OS error code does not reach the caller. What we
        // can pin is that it is NOT a timeout - i.e. the fast fail, not the retry loop.
        let text = error.to_string();
        assert!(
            !text.contains("timed out") && !text.contains("deadline"),
            "a missing pipe must fail immediately, not by exhausting a deadline: {text}"
        );
        let started = std::time::Instant::now();
        let _ = connect_local(Some(Duration::from_secs(5))).await;
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "a missing pipe must not be retried until the timeout"
        );

        // The no-timeout arm is the other half of the same `match`.
        assert!(connect_local(None).await.is_err());

        match previous {
            Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
            None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
        }
    }

    /// A channel that cannot be built must fail with every endpoint it tried, so the
    /// operator can see WHICH one is misconfigured. This is also the only path that
    /// exercises the per-request-timeout arm of the TCP endpoint.
    ///
    /// The failure text is deliberately not asserted beyond the endpoint name: a connect
    /// to a closed local port can elapse a short budget (measured: a 500 ms budget timed
    /// out before the refusal surfaced on this loaded box), so "refused" vs "timed out"
    /// is not a stable discriminator. The endpoint label is, and the tiny budget keeps
    /// the deadline arm reachable here; which of the two arms fires is left to the run.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_channel_that_cannot_be_built_names_every_endpoint_it_tried() {
        let error = connect_channel(
            Some("http://127.0.0.1:1"),
            Duration::from_millis(1),
            Some(Duration::from_secs(5)),
        )
        .await
        .err()
        .unwrap();
        assert!(
            error.details.contains("unable to connect to Future Agent"),
            "the failure must say what could not be reached: {}",
            error.details
        );
        assert!(
            error.details.contains("127.0.0.1:1"),
            "the failed endpoint must be named: {}",
            error.details
        );
    }
    /// A supplied resolver is the *only* lookup for a TCP endpoint: a name
    /// that does not exist anywhere connects when the resolver knows where it
    /// lives, and the same endpoint fails without one. Both halves matter —
    /// the first proves the seam is used, the second proves the default path
    /// is still `getaddrinfo` (a test that only asserts success would also
    /// pass if the resolver were ignored and the name happened to resolve).
    #[tokio::test]
    async fn a_supplied_resolver_decides_where_a_tcp_endpoint_connects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                drop(stream);
            }
        });

        let resolver: TcpNameResolver = Arc::new(move |host: String, asked: u16| {
            assert_eq!(host, "agent.invalid", "the resolver gets the endpoint host");
            assert_eq!(asked, port, "and the endpoint port");
            Box::pin(async move { Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))]) })
        });
        let configured = format!("agent.invalid:{port}");

        let connected = connect_pinned_channel_with(
            Some(&configured),
            Duration::from_secs(5),
            None,
            Some(resolver),
        )
        .await;
        assert!(
            connected.is_ok(),
            "the resolver was not consulted: {:?}",
            connected.err()
        );

        let without = connect_pinned_channel(Some(&configured), Duration::from_secs(5), None).await;
        assert!(
            without.is_err(),
            "without a resolver this name must still go through getaddrinfo and fail"
        );
    }

    /// A pinned HTTPS endpoint has no configured TLS connector, so the client
    /// must refuse it rather than silently speaking plaintext to it.
    #[tokio::test]
    async fn a_pinned_https_endpoint_is_refused_without_a_tls_connector() {
        let error = connect_pinned_channel(
            Some("https://127.0.0.1:443"),
            Duration::from_millis(250),
            None,
        )
        .await
        .err()
        .expect("an https agent endpoint must not be connected in plaintext");
        assert!(
            error.details.contains("TLS connector"),
            "the refusal must name the missing connector: {}",
            error.details
        );
    }

    /// An endpoint that never answers must not hang discovery: the attempt is
    /// bounded by a connect budget and reports the endpoint it tried.
    ///
    /// A pending connector models stalled establishment without relying on
    /// routing, firewalls or a peer. Tokio's paused clock makes the deadline
    /// exact and avoids wall-clock timing assumptions on loaded CI hosts.
    #[tokio::test(start_paused = true)]
    async fn an_endpoint_that_never_answers_is_bounded_by_the_connect_budget() {
        let budget = Duration::from_millis(300);
        let address = "http://127.0.0.1:12345";
        let attempts = std::cell::Cell::new(0);
        let started = tokio::time::Instant::now();
        let error = connect_channel_with_connector(Some(address), budget, |endpoint| {
            assert_eq!(endpoint, AgentEndpoint::Tcp(address.into()));
            attempts.set(attempts.get() + 1);
            std::future::pending()
        })
        .await
        .err()
        .expect("a pending connector must time out instead of producing a channel");
        assert_eq!(attempts.get(), 1);
        assert_eq!(error.kind, Some(io::ErrorKind::TimedOut));
        assert!(
            error.details.contains(address),
            "the failure must name the endpoint it tried: {}",
            error.details
        );
        assert_eq!(started.elapsed(), budget);
    }

    /// The accepted local IO is the server's whole transport on this platform:
    /// reading, writing, flushing and shutting down must all delegate to the
    /// socket, and tonic asks for the connect info before it serves the stream.
    /// A silent failure in any of them would break every local RPC.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_accepted_local_io_round_trips_bytes_and_reports_its_connect_info() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("agent.sock");
        let mut incoming = bind_local_at(socket.clone()).await.unwrap();
        let mut client = tokio::net::UnixStream::connect(&socket).await.unwrap();
        let mut server = std::pin::Pin::new(&mut incoming)
            .next()
            .await
            .expect("the endpoint must accept the connection")
            .expect("the accepted IO must be usable");
        assert_eq!(
            <LocalIo as tonic::transport::server::Connected>::connect_info(&server),
            ()
        );

        client.write_all(b"ping").await.unwrap();
        client.flush().await.unwrap();
        let mut buf = [0_u8; 4];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");

        server.write_all(b"pong").await.unwrap();
        server.flush().await.unwrap();
        let mut reply = [0_u8; 4];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"pong");

        server.shutdown().await.unwrap();
        drop(client);
    }

    /// The default (unconfigured) endpoint is the socket under this instance's
    /// own FutureOS home, and reserve → bind → connect must work as a chain.
    /// `FUTURE_HOME` is process-global, so this test serializes on the crate's
    /// environment lock (see `crate::home::env_guard`).
    #[cfg(unix)]
    #[tokio::test]
    // The environment lock is held across the awaits on purpose (the endpoint stays
    // redirected for the whole chain) on a current-thread runtime, so no task shares
    // the thread with it.
    #[allow(clippy::await_holding_lock)]
    async fn the_default_endpoint_reserves_binds_and_connects_under_its_own_home() {
        let _guard = crate::home::env_guard();
        let home = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
        let previous_socket = std::env::var_os("FUTURE_AGENT_SOCKET");
        std::env::remove_var("FUTURE_AGENT_SOCKET");
        std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

        let socket = home.path().join("run").join("agent.sock");
        let reserved = reserve_local_endpoint()
            .await
            .expect("the instance home must own a reservable endpoint");
        assert!(
            reserve_local_endpoint().await.is_err(),
            "a reservation is exclusive"
        );
        drop(reserved);

        let mut incoming = bind_local().await.expect("the endpoint must bind");
        assert!(
            socket.exists(),
            "the endpoint lives under the instance home"
        );

        let connected = connect_channel(None, Duration::from_secs(1), Some(Duration::from_secs(5)))
            .await
            .expect("the default endpoint must connect without configuration");
        assert!(matches!(connected.endpoint, AgentEndpoint::Local));
        let accepted = std::pin::Pin::new(&mut incoming).next().await.unwrap();
        assert!(accepted.is_ok(), "the client's connection must be accepted");
        drop(connected);
        drop(incoming);
        assert!(!socket.exists(), "teardown removes the endpoint");

        match previous_socket {
            Some(value) => std::env::set_var("FUTURE_AGENT_SOCKET", value),
            None => std::env::remove_var("FUTURE_AGENT_SOCKET"),
        }
        match previous_home {
            Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
            None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
        }
    }

    /// The socket directory is prepared before binding, and the failure has to
    /// tell an operator what to do about it (the common cause is a FutureOS home
    /// on a filesystem that cannot host a socket).
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unusable_socket_directory_names_the_directory_and_the_way_out() {
        let temp = tempfile::tempdir().unwrap();
        let occupant = temp.path().join("not-a-directory");
        std::fs::write(&occupant, "file").unwrap();
        let error = bind_local_at(occupant.join("agent.sock"))
            .await
            .err()
            .expect("a socket below a regular file cannot bind");
        let message = error.to_string();
        assert!(
            message.contains("cannot prepare local socket directory"),
            "{message}"
        );
        assert!(message.contains("FUTURE_HOME"), "{message}");
        assert_eq!(std::fs::read_to_string(&occupant).unwrap(), "file");
    }
}
