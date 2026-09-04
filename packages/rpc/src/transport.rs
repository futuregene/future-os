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

use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_stream::Stream;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

/// Sentinel used by CLI/config surfaces that want automatic local discovery.
pub const AUTO_ENDPOINT: &str = "auto";

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

/// Build the ordered connection plan. `None`, an empty value, and `auto`
/// select only the per-user local transport. A configured TCP endpoint is
/// tried first, with local IPC as a compatibility fallback.
pub fn connection_plan(configured: Option<&str>) -> Vec<AgentEndpoint> {
    let configured = configured
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(AUTO_ENDPOINT));
    match configured {
        Some(addr) => vec![AgentEndpoint::Tcp(addr.to_string()), AgentEndpoint::Local],
        None => vec![AgentEndpoint::Local],
    }
}

/// Connect to the first reachable endpoint in the discovery plan.
pub async fn connect_channel(
    configured: Option<&str>,
    connect_timeout: Duration,
    request_timeout: Duration,
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
    request_timeout: Duration,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    match endpoint {
        AgentEndpoint::Tcp(addr) => {
            let endpoint = Endpoint::from_shared(normalize_tcp_uri(addr))?
                .connect_timeout(connect_timeout)
                .timeout(request_timeout);
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
    if let Some(path) = std::env::var_os("FUTURE_AGENT_SOCKET").filter(|v| !v.is_empty()) {
        return path.into();
    }
    #[cfg(target_os = "linux")]
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return std::path::PathBuf::from(runtime)
            .join("future")
            .join("agent.sock");
    }
    let home = std::env::var_os("HOME")
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
    request_timeout: Duration,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    let path = local_socket_path();
    let endpoint = Endpoint::try_from("http://future-agent.local")?.timeout(request_timeout);
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
    request_timeout: Duration,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name = local_pipe_name();
    let endpoint = Endpoint::try_from("http://future-agent.local")?.timeout(request_timeout);
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
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
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
    let sid = current_user_sid_string().unwrap_or_else(|_| "unknown-user".to_string());
    format!(r"\\.\pipe\future-agent-{sid}")
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
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows_sys::Win32::Security::{SDDL_REVISION_1, SECURITY_ATTRIBUTES};

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
    use tokio_stream::StreamExt;

    #[test]
    fn automatic_mode_never_falls_back_to_the_shared_tcp_port() {
        assert_eq!(connection_plan(None), vec![AgentEndpoint::Local]);
        assert_eq!(connection_plan(Some("")), vec![AgentEndpoint::Local]);
        assert_eq!(connection_plan(Some("auto")), vec![AgentEndpoint::Local]);
    }

    #[test]
    fn explicitly_configured_tcp_is_tried_before_local_ipc() {
        assert_eq!(
            connection_plan(Some("http://127.0.0.1:50051")),
            vec![
                AgentEndpoint::Tcp("http://127.0.0.1:50051".into()),
                AgentEndpoint::Local,
            ]
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
}
