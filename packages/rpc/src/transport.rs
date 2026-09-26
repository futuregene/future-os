//! Per-user local transport discovery for the Future Agent.
//!
//! Native local IPC is the default security boundary: Unix-domain sockets are
//! created below a mode-0700 directory and accept only the current UID;
//! Windows named pipes carry a protected DACL for the current user. TCP is
//! available only when a caller explicitly supplies an address.

use std::fmt;
use std::io;
use std::pin::Pin;
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
    let mut failures = Vec::new();
    for endpoint in connection_plan(configured) {
        let label = endpoint.label();
        let attempt = tokio::time::timeout(
            connect_timeout,
            connect_one(&endpoint, connect_timeout, request_timeout),
        )
        .await;
        match attempt {
            Ok(Ok(channel)) => return Ok(ConnectedChannel { endpoint, channel }),
            Ok(Err(error)) => failures.push(format!("{label}: {error}")),
            Err(_) => failures.push(format!("{label}: connection timed out")),
        }
    }
    Err(ConnectError {
        details: format!(
            "unable to connect to Future Agent ({})",
            failures.join("; ")
        ),
    })
}

async fn connect_one(
    endpoint: &AgentEndpoint,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    match endpoint {
        AgentEndpoint::Tcp(addr) => {
            let endpoint =
                Endpoint::from_shared(normalize_tcp_uri(addr))?.connect_timeout(connect_timeout);
            let endpoint = match request_timeout {
                Some(timeout) => endpoint.timeout(timeout),
                None => endpoint,
            };
            Ok(endpoint.connect().await?)
        }
        AgentEndpoint::Local => connect_local(request_timeout).await,
    }
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
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HOME"),
    )
}

/// Local endpoint of the instance that owns `future_home` (see
/// [`crate::home`]), with every environment input injected for testability.
///
/// A redirected FutureOS home owns its own endpoint under `<home>/run`, in
/// front of the shared `$XDG_RUNTIME_DIR`: the runtime directory is per-user,
/// not per-instance, so a second isolated Agent binding there would steal the
/// default instance's socket instead of running beside it.
#[cfg(unix)]
fn local_socket_path_from(
    explicit: Option<std::ffi::OsString>,
    future_home: Option<std::path::PathBuf>,
    xdg_runtime_dir: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> std::path::PathBuf {
    if let Some(path) = explicit.filter(|v| !v.is_empty()) {
        return path.into();
    }
    if let Some(future_home) = future_home {
        return future_home.join("run").join("agent.sock");
    }
    #[cfg(target_os = "linux")]
    if let Some(runtime) = xdg_runtime_dir.filter(|v| !v.is_empty()) {
        return std::path::PathBuf::from(runtime)
            .join("future")
            .join("agent.sock");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = xdg_runtime_dir;
    let home = home
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".future").join("run").join("agent.sock")
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

#[cfg(unix)]
async fn connect_local(
    request_timeout: Option<Duration>,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    let path = local_socket_path();
    let endpoint = Endpoint::try_from("http://future-agent.local")?;
    let endpoint = match request_timeout {
        Some(timeout) => endpoint.timeout(timeout),
        None => endpoint,
    };
    let channel = endpoint
        .connect_with_connector(service_fn(move |_| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(TokioIo::new)
            }
        }))
        .await?;
    Ok(channel)
}

#[cfg(windows)]
async fn connect_local(
    request_timeout: Option<Duration>,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name = local_pipe_name();
    let endpoint = Endpoint::try_from("http://future-agent.local")?;
    let endpoint = match request_timeout {
        Some(timeout) => endpoint.timeout(timeout),
        None => endpoint,
    };
    let channel = endpoint
        .connect_with_connector(service_fn(move |_| {
            let pipe_name = pipe_name.clone();
            async move {
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

#[cfg(unix)]
struct SocketCleanup(std::path::PathBuf);

#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Bind the current user's native local endpoint.
#[cfg(unix)]
pub async fn bind_local() -> io::Result<LocalIncoming> {
    bind_local_at(local_socket_path()).await
}

#[cfg(unix)]
async fn bind_local_at(path: std::path::PathBuf) -> io::Result<LocalIncoming> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "local socket has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let metadata = std::fs::metadata(parent)?;
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "local socket directory is not owned by the current user: {}",
                parent.display()
            ),
        ));
    }
    // Validate ownership before mutating an existing directory's permissions.
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to replace untrusted local endpoint: {}",
                    path.display()
                ),
            ));
        }
        std::fs::remove_file(&path)?;
    }

    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let expected_uid = unsafe { libc::geteuid() };
    let cleanup_path = path.clone();
    let incoming = async_stream::stream! {
        let _cleanup = SocketCleanup(cleanup_path);
        loop {
            match listener.accept().await {
                Ok((stream, _)) => match stream.peer_cred() {
                    Ok(cred) if cred.uid() == expected_uid => yield Ok(LocalIo { inner: stream }),
                    Ok(cred) => {
                        eprintln!("Future Agent rejected local IPC peer uid {} (expected {})", cred.uid(), expected_uid);
                    }
                    Err(error) => yield Err(error),
                },
                Err(error) => yield Err(error),
            }
        }
    };
    Ok(Box::pin(incoming))
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
            local_pipe_name(),
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    };
    unsafe { LocalFree(descriptor.cast()) };
    result
}

#[cfg(windows)]
pub async fn bind_local() -> io::Result<LocalIncoming> {
    let first = create_protected_pipe(true)?;
    let incoming = async_stream::stream! {
        let mut server = first;
        loop {
            match server.connect().await {
                Ok(()) => match create_protected_pipe(false) {
                    Ok(next) => {
                        let connected = std::mem::replace(&mut server, next);
                        yield Ok(LocalIo { inner: connected });
                    }
                    Err(error) => yield Err(error),
                },
                Err(error) => yield Err(error),
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
            Some("/run/user/1000".into()),
            Some("/home/user".into()),
        );
        assert_eq!(path, std::path::PathBuf::from("/tmp/explicit.sock"));
    }

    /// The XDG runtime dir is per-user, so an instance with its own FutureOS
    /// home must not bind there — it would take over the default instance's
    /// socket. Regression guard for multi-instance isolation.
    #[cfg(unix)]
    #[test]
    fn redirected_future_home_owns_its_endpoint_and_ignores_xdg() {
        let home = std::path::PathBuf::from("/tmp/futureos-home");
        let path = local_socket_path_from(
            None,
            Some(home.clone()),
            Some("/run/user/1000".into()),
            Some("/home/user".into()),
        );
        assert_eq!(path, home.join("run").join("agent.sock"));
    }

    #[cfg(unix)]
    #[test]
    fn without_a_redirect_the_endpoint_keeps_its_documented_chain() {
        #[cfg(target_os = "linux")]
        assert_eq!(
            local_socket_path_from(
                None,
                None,
                Some("/run/user/1000".into()),
                Some("/home/user".into()),
            ),
            std::path::PathBuf::from("/run/user/1000/future/agent.sock")
        );
        assert_eq!(
            local_socket_path_from(
                None,
                None,
                Some(String::new().into()),
                Some("/home/user".into()),
            ),
            std::path::PathBuf::from("/home/user/.future/run/agent.sock")
        );
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

    /// End-to-end over a REAL named pipe, with an isolated `FUTURE_HOME` so the test
    /// never binds the default instance's endpoint (a running agent owns that one, and
    /// `first_pipe_instance` would make this fail or, worse, interfere).
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

        // Unique per run: the pipe name is derived from this path.
        let home = tempfile::tempdir().unwrap();
        let previous = std::env::var_os(crate::home::FUTURE_HOME_ENV);
        std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

        let mut incoming = bind_local().await.expect("the pipe must bind");
        let pipe = local_pipe_name();
        assert!(
            pipe.contains(&crate::home::home_tag(home.path())),
            "the test must bind its OWN pipe, not the default instance's: {pipe}"
        );

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

        match previous {
            Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
            None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
        }
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
    /// is not a stable discriminator. The endpoint label is.
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
            error.details.contains("connection timed out"),
            "a 1 ms budget cannot accommodate a TCP attempt, so the deadline must \
             fire first: {}",
            error.details
        );
        assert!(
            error.details.contains("unable to connect to Future Agent"),
            "the failure must say what could not be reached: {}",
            error.details
        );
        assert!(
            error.details.contains("127.0.0.1:1"),
            "the timed-out endpoint must be named: {}",
            error.details
        );
    }
}
