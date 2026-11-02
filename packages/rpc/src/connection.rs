//! Readiness, physical connection ownership and permission to start locally.
//! A failed business handshake never authorizes startup or a mutation.
use crate::{payloads_ext::AgentReadinessPayload, proto, transport};
use std::{
    fmt, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tonic::transport::Channel;

pub const AGENT_PROTOCOL_VERSION: u32 = 2;
pub const AGENT_STORAGE_SCHEMA_VERSION: u32 = 8;
/// Coordinated Agent release expected by RPC consumers. This differs from the
/// CLI/TUI display version in bare development builds; suffixes are ignored.
pub const EXPECTED_AGENT_VERSION: &str = env!("EXPECTED_AGENT_VERSION");
pub const EXPECTED_INSTANCE_HEADER: &str = "x-future-agent-instance";
pub const EXPECTED_DATA_ROOT_HEADER: &str = "x-future-agent-data-root";

/// Protocol 2 requires a complete client contract on every business request.
/// These headers fence instances; local IPC permissions remain authentication.
pub const CLIENT_VERSION_HEADER: &str = "x-future-client-version";
pub const CLIENT_PROTOCOL_HEADER: &str = "x-future-client-protocol";
pub const CLIENT_SCHEMA_HEADER: &str = "x-future-client-storage-schema";

#[allow(clippy::result_large_err)]
pub fn bind_request<T>(
    mut request: tonic::Request<T>,
    readiness: &AgentReadinessPayload,
    version: &str,
) -> Result<tonic::Request<T>, tonic::Status> {
    for (header, value) in [
        (
            EXPECTED_INSTANCE_HEADER,
            readiness.agent_instance_id.clone(),
        ),
        (EXPECTED_DATA_ROOT_HEADER, readiness.data_root_id.clone()),
        (CLIENT_VERSION_HEADER, version.to_owned()),
        (CLIENT_PROTOCOL_HEADER, AGENT_PROTOCOL_VERSION.to_string()),
        (
            CLIENT_SCHEMA_HEADER,
            AGENT_STORAGE_SCHEMA_VERSION.to_string(),
        ),
    ] {
        request.metadata_mut().insert(
            header,
            value.parse().map_err(|_| {
                tonic::Status::failed_precondition("invalid client connection contract")
            })?,
        );
    }
    Ok(request)
}

/// Validate before dispatch or session hydration, including read-only business
/// calls and subscriptions. Only readiness discovery is exempt.
#[allow(clippy::result_large_err)]
pub fn validate_request_metadata(
    metadata: &tonic::metadata::MetadataMap,
    version: &str,
    instance: &str,
    root: &str,
) -> Result<(), tonic::Status> {
    let required = |header: &'static str| {
        metadata
            .get(header)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                tonic::Status::failed_precondition(
                    "missing client contract; reconnect with a matching FutureOS client",
                )
            })
    };
    if required(EXPECTED_INSTANCE_HEADER)? != instance
        || required(EXPECTED_DATA_ROOT_HEADER)? != root
        || release_core(version).is_none()
        || release_core(required(CLIENT_VERSION_HEADER)?) != release_core(version)
        || required(CLIENT_PROTOCOL_HEADER)?.parse::<u32>().ok() != Some(AGENT_PROTOCOL_VERSION)
        || required(CLIENT_SCHEMA_HEADER)?.parse::<u32>().ok() != Some(AGENT_STORAGE_SCHEMA_VERSION)
    {
        return Err(tonic::Status::failed_precondition(
            "Agent instance, data root or client contract changed; reconnect with a matching FutureOS client",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct CompatibilityRequirements {
    pub version: String,
    pub expected_home: Option<PathBuf>,
}

impl CompatibilityRequirements {
    pub fn for_endpoint(configured: &str, version: &str) -> Self {
        Self {
            version: version.to_owned(),
            expected_home: matches!(
                transport::connection_plan(Some(configured))[0],
                transport::AgentEndpoint::Local
            )
            .then(crate::home::future_home),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ConnectionError {
    Transport(transport::ConnectError),
    Unverified(String),
    Incompatible {
        agent_version: Option<String>,
        reason: String,
    },
    InstanceChanged,
    StartupBlocked(String),
    /// Another process holds this data root's Agent instance lock. It is not a
    /// failure: the holder is starting (or serving) the Agent, and a client that
    /// meets this must wait for it rather than start a second one. Kept distinct
    /// from [`ConnectionError::StartupBlocked`] so supervision can tell "yield"
    /// from "report and stop" without reading the message.
    InstanceBusy(String),
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(f),
            Self::Unverified(reason) => write!(
                f,
                "Agent readiness could not be verified: {reason}; local startup is forbidden"
            ),
            Self::Incompatible { reason, .. } => write!(
                f,
                "Incompatible Future Agent: {reason}; local startup is forbidden"
            ),
            Self::InstanceChanged => f.write_str(
                "Agent instance changed; reconnect and verify readiness before sending commands",
            ),
            Self::StartupBlocked(reason) => write!(f, "Cannot start a local Agent: {reason}"),
            Self::InstanceBusy(reason) => write!(
                f,
                "Cannot start a local Agent: {reason}\nAnother process owns this data root; \
                 waiting for it instead of starting a second Agent"
            ),
        }
    }
}
impl std::error::Error for ConnectionError {}

impl ConnectionError {
    /// The local instance lock is held by another process. Distinct from every
    /// other refusal because the right answer is to wait for that process, not
    /// to report a fault or to start a second Agent.
    pub fn is_instance_busy(&self) -> bool {
        matches!(self, Self::InstanceBusy(_))
    }
}

/// Hash a canonical directory, not its database contents. This identifies a
/// data root without publishing a user's path or creating/modifying storage.
pub fn data_root_id(home: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let home = home.canonicalize()?;
    #[cfg(windows)]
    let bytes = home.to_string_lossy().to_lowercase().into_bytes();
    #[cfg(not(windows))]
    let bytes = home.as_os_str().as_encoded_bytes().to_vec();
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn release_core(version: &str) -> Option<&str> {
    let core = version.split(['-', '+']).next()?;
    let parts: Vec<_> = core.split('.').collect();
    (parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())))
    .then_some(core)
}

pub fn validate_readiness(
    info: &AgentReadinessPayload,
    expected: &CompatibilityRequirements,
) -> Result<(), ConnectionError> {
    let incompatible = |reason| ConnectionError::Incompatible {
        agent_version: (!info.version.is_empty()).then(|| info.version.clone()),
        reason,
    };
    if info.version.is_empty()
        || info.agent_instance_id.is_empty()
        || !info.agent_instance_id.is_ascii()
        || info.agent_instance_id.len() > 256
        || info
            .agent_instance_id
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(incompatible(
            "incomplete readiness response (version or instance identity)".into(),
        ));
    }
    if release_core(&expected.version).is_none()
        || release_core(&info.version) != release_core(&expected.version)
    {
        return Err(incompatible(format!(
            "Agent version {} does not match client {}",
            info.version, expected.version
        )));
    }
    if info.protocol_version != AGENT_PROTOCOL_VERSION
        || info.storage_schema_version != AGENT_STORAGE_SCHEMA_VERSION
    {
        return Err(incompatible(format!(
            "protocol/schema {}/{} does not match required {}/{}",
            info.protocol_version,
            info.storage_schema_version,
            AGENT_PROTOCOL_VERSION,
            AGENT_STORAGE_SCHEMA_VERSION
        )));
    }
    if info.data_root_id.len() != 64
        || !info
            .data_root_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(incompatible(
            "missing or invalid Agent data-root identity".into(),
        ));
    }
    if let Some(home) = &expected.expected_home {
        let root = data_root_id(home).map_err(|error| {
            incompatible(format!("cannot establish the expected data root: {error}"))
        })?;
        if info.data_root_id != root {
            return Err(incompatible("Agent serves a different data root".into()));
        }
    }
    Ok(())
}

async fn read_readiness(
    inner: &mut proto::future_agent_client::FutureAgentClient<Channel>,
    timeout: Duration,
) -> Result<AgentReadinessPayload, ConnectionError> {
    let mut request = tonic::Request::new(proto::RpcCommand {
        id: "readiness-gate".into(),
        r#type: "get_agent_readiness".into(),
        ..Default::default()
    });
    request.set_timeout(timeout);
    let response = tokio::time::timeout(timeout, inner.execute_command(request))
        .await
        .map_err(|_| ConnectionError::Unverified("handshake timed out".into()))?
        .map_err(|error| {
            if error.code() == tonic::Code::Unimplemented {
                ConnectionError::Incompatible {
                    agent_version: None,
                    reason: "Agent does not implement readiness".into(),
                }
            } else {
                ConnectionError::Unverified(error.to_string())
            }
        })?
        .into_inner();
    if !response.success {
        if response.error_code == "agent_stopping" {
            return Err(ConnectionError::StartupBlocked(
                "Agent service is stopping; wait for it to exit before reconnecting; no local fallback".into(),
            ));
        }
        return Err(ConnectionError::Incompatible {
            agent_version: None,
            reason: format!("readiness rejected: {}", response.error),
        });
    }
    serde_json::from_value(crate::decode::response_data(&response)).map_err(|error| {
        ConnectionError::Incompatible {
            agent_version: None,
            reason: format!("incomplete readiness response: {error}"),
        }
    })
}

/// Only this wrapper exposes business RPCs after readiness. Its channel is
/// physically pinned; transparent reconnect to an older server is forbidden.
#[derive(Clone, Debug)]
pub struct VerifiedAgentClient {
    inner: proto::future_agent_client::FutureAgentClient<Channel>,
    requirements: CompatibilityRequirements,
    readiness: AgentReadinessPayload,
    handshake_timeout: Duration,
}

impl VerifiedAgentClient {
    pub fn readiness(&self) -> &AgentReadinessPayload {
        &self.readiness
    }

    pub async fn verify(&mut self) -> Result<(), ConnectionError> {
        let info = read_readiness(&mut self.inner, self.handshake_timeout).await?;
        validate_readiness(&info, &self.requirements)?;
        if info.agent_instance_id != self.readiness.agent_instance_id
            || info.data_root_id != self.readiness.data_root_id
        {
            return Err(ConnectionError::InstanceChanged);
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)] // Tonic request APIs expose Status directly.
    fn bind<T>(&self, request: tonic::Request<T>) -> Result<tonic::Request<T>, tonic::Status> {
        bind_request(request, &self.readiness, &self.requirements.version)
    }

    #[allow(clippy::result_large_err)] // Tonic request APIs expose Status directly.
    pub async fn execute_command(
        &mut self,
        request: impl tonic::IntoRequest<proto::RpcCommand>,
    ) -> Result<tonic::Response<proto::RpcResponse>, tonic::Status> {
        self.verify().await.map_err(connection_status)?;
        let request = self.bind(request.into_request())?;
        self.inner.execute_command(request).await
    }

    #[allow(clippy::result_large_err)] // Tonic request APIs expose Status directly.
    pub async fn stream_events(
        &mut self,
        request: impl tonic::IntoRequest<proto::StreamRequest>,
    ) -> Result<tonic::Response<tonic::Streaming<proto::StreamEvent>>, tonic::Status> {
        self.verify().await.map_err(connection_status)?;
        let request = self.bind(request.into_request())?;
        self.inner.stream_events(request).await
    }
}

fn connection_status(error: ConnectionError) -> tonic::Status {
    match error {
        ConnectionError::Incompatible { .. } | ConnectionError::InstanceChanged => {
            tonic::Status::failed_precondition(error.to_string())
        }
        _ => tonic::Status::unavailable(error.to_string()),
    }
}

pub async fn connect_verified(
    configured: &str,
    version: &str,
    connect_timeout: Duration,
    handshake_timeout: Duration,
) -> Result<VerifiedAgentClient, ConnectionError> {
    connect_verified_with(
        configured,
        version,
        connect_timeout,
        handshake_timeout,
        None,
    )
    .await
}

/// [`connect_verified`] with a hostname resolver of the caller's own.
///
/// `resolver: None` is byte-for-byte the behaviour above; a caller supplies one
/// only when this process cannot rely on libc's resolver (a static musl build
/// on a system with no `/etc/resolv.conf`). The desktop, the TUI and the host
/// all call [`connect_verified`] and keep resolving through `getaddrinfo`.
pub async fn connect_verified_with(
    configured: &str,
    version: &str,
    connect_timeout: Duration,
    handshake_timeout: Duration,
    resolver: Option<transport::TcpNameResolver>,
) -> Result<VerifiedAgentClient, ConnectionError> {
    let connected =
        transport::connect_pinned_channel_with(Some(configured), connect_timeout, None, resolver)
            .await
            .map_err(ConnectionError::Transport)?;
    let mut inner = transport::agent_client(connected.channel);
    let requirements = CompatibilityRequirements::for_endpoint(configured, version);
    let readiness = read_readiness(&mut inner, handshake_timeout).await?;
    validate_readiness(&readiness, &requirements)?;
    Ok(VerifiedAgentClient {
        inner,
        requirements,
        readiness,
        handshake_timeout,
    })
}

pub enum AgentSelection {
    External(Box<VerifiedAgentClient>),
    LocalStart(LocalStartPermit),
}

/// Which endpoint a start permit covers.
///
/// The local IPC endpoint is derived from the environment, so it is captured as
/// the label in force at discovery time and re-read by
/// [`LocalStartPermit::recheck`]. A TCP endpoint is the value the caller passed
/// in: nothing in the environment can move it, so there is nothing to re-read.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PermitEndpoint {
    Local(String),
    Tcp(std::net::SocketAddr),
}

/// A short-lived preflight permission, never a replacement for the Agent's
/// authoritative instance lock. The service re-acquires that lock before any
/// storage initialization; concurrent startup may safely lose that race.
pub struct LocalStartPermit {
    home: PathBuf,
    endpoint: PermitEndpoint,
}
impl LocalStartPermit {
    pub fn future_home(&self) -> &Path {
        &self.home
    }
    /// Whether this permit covers serving the Agent on `ip:port`.
    ///
    /// The Agent asks this before it binds an explicit TCP endpoint: a permit
    /// obtained for the local IPC endpoint does not authorize one, and a permit
    /// for one address does not authorize another.
    pub fn serves_tcp(&self, ip: &str, port: u16) -> bool {
        let Ok(ip) = ip.parse::<std::net::IpAddr>() else {
            return false;
        };
        matches!(&self.endpoint, PermitEndpoint::Tcp(bound) if bound.port() == port && bound.ip() == ip)
    }
    pub fn recheck(&self) -> Result<(), ConnectionError> {
        let configuration_changed = match &self.endpoint {
            PermitEndpoint::Local(label) => *label != transport::local_endpoint_label(),
            PermitEndpoint::Tcp(_) => false,
        };
        if configuration_changed || self.home != crate::home::future_home() {
            return Err(ConnectionError::StartupBlocked(
                "Agent discovery configuration changed".into(),
            ));
        }
        check_instance_lock(&self.home)
    }
}

pub fn check_instance_lock(home: &Path) -> Result<(), ConnectionError> {
    crate::home::ensure_restore_complete(home)
        .map_err(|error| ConnectionError::StartupBlocked(error.to_string()))?;
    let path = home.join("agent").join("agent-instance.lock");
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ConnectionError::StartupBlocked(format!(
                "instance lock unavailable: {error}"
            )))
        }
    };
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.try_write().map_err(|error| {
        // A held lock gets the same owner-identifying report the Agent itself
        // prints on a conflicting start: pid, host, executable and the advice
        // for shared-home setups. A held lock is a *yield* signal, not a
        // failure, so it keeps its own variant; anything else stays a plain I/O
        // complaint that no amount of waiting will fix.
        if error.kind() == io::ErrorKind::WouldBlock {
            return ConnectionError::InstanceBusy(crate::instance_lock::lock_conflict_message(
                &path, home,
            ));
        }
        ConnectionError::StartupBlocked(format!(
            "instance lock is occupied or unavailable: {error}; do not start another Agent"
        ))
    })?;
    Ok(())
}

/// Select an Agent for a caller that may only attach to one that answers.
pub async fn select_agent(
    configured: &str,
    version: &str,
    connect_timeout: Duration,
    handshake_timeout: Duration,
) -> Result<AgentSelection, ConnectionError> {
    select_with(
        configured,
        version,
        connect_timeout,
        handshake_timeout,
        BindPermission::AttachOnly,
    )
    .await
}

/// Select an Agent for a host that **serves** it at `configured`.
///
/// Every case behaves like [`select_agent`] except one: an explicit TCP address
/// that nothing answers on is this host's own to fill, so it yields a start
/// permit for exactly that address instead of the "no local fallback" refusal.
/// Only a loopback address qualifies — binding a remote name is not something
/// this process can do, and quietly starting a local Agent for an address that
/// was meant to be remote is the redirection [`select_agent`] refuses.
pub async fn select_agent_serving(
    configured: &str,
    version: &str,
    connect_timeout: Duration,
    handshake_timeout: Duration,
) -> Result<AgentSelection, ConnectionError> {
    select_with(
        configured,
        version,
        connect_timeout,
        handshake_timeout,
        BindPermission::Serve,
    )
    .await
}

/// What the caller may do when nothing answers at the configured endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindPermission {
    /// Attach only: an explicit TCP endpoint that is unreachable is a refusal.
    AttachOnly,
    /// This host serves the Agent at that endpoint, so it may start one there.
    Serve,
}

async fn select_with(
    configured: &str,
    version: &str,
    connect_timeout: Duration,
    handshake_timeout: Duration,
    bind: BindPermission,
) -> Result<AgentSelection, ConnectionError> {
    let verified = connect_verified(configured, version, connect_timeout, handshake_timeout).await;
    select_from(configured, verified, bind)
}

/// The decision after one connect attempt: attach, or decide whether this caller
/// may start an Agent at the endpoint instead.
///
/// Split out of [`select_with`] so the decision can be driven with the exact
/// failure a platform reports for an endpoint nothing serves — on Windows a
/// connect timeout rather than a refusal (see [`nothing_answers`]) — without
/// reproducing that platform.
fn select_from(
    configured: &str,
    verified: Result<VerifiedAgentClient, ConnectionError>,
    bind: BindPermission,
) -> Result<AgentSelection, ConnectionError> {
    match verified {
        Ok(client) => Ok(AgentSelection::External(Box::new(client))),
        Err(ConnectionError::Transport(error)) => {
            let endpoint = match (
                transport::connection_plan(Some(configured))[0].clone(),
                bind,
            ) {
                (transport::AgentEndpoint::Local, _) => {
                    if bind == BindPermission::Serve {
                        return Err(ConnectionError::StartupBlocked(
                            "serving the Agent needs an explicit TCP address, not automatic discovery"
                                .into(),
                        ));
                    }
                    PermitEndpoint::Local(transport::local_endpoint_label())
                }
                (transport::AgentEndpoint::Tcp(_), BindPermission::AttachOnly) => {
                    return Err(ConnectionError::StartupBlocked(format!(
                        "configured external endpoint is unavailable: {error}; no local fallback"
                    )));
                }
                (transport::AgentEndpoint::Tcp(addr), BindPermission::Serve) => {
                    let bound = transport::parse_bind_addr(&addr)
                        .map_err(|error| ConnectionError::StartupBlocked(error.to_string()))?;
                    if !bound.ip().is_loopback() {
                        return Err(ConnectionError::StartupBlocked(format!(
                            "cannot serve an Agent on {addr}: only a loopback address is this host's to bind"
                        )));
                    }
                    PermitEndpoint::Tcp(bound)
                }
            };
            let home = crate::home::future_home();
            check_instance_lock(&home)?;
            if !nothing_answers(&error, &endpoint) {
                // Still no permission to launch. Preserve the transport class
                // so an owning host can repair its own unresponsive service.
                return Err(ConnectionError::Transport(error));
            }
            #[cfg(unix)]
            if matches!(endpoint, PermitEndpoint::Local(_)) {
                if let Ok(metadata) = std::fs::symlink_metadata(transport::local_socket_path()) {
                    use std::os::unix::fs::FileTypeExt;
                    if !metadata.file_type().is_socket() {
                        return Err(ConnectionError::StartupBlocked(
                            "local endpoint is occupied by a non-socket file".into(),
                        ));
                    }
                }
            }
            Ok(AgentSelection::LocalStart(LocalStartPermit {
                home,
                endpoint,
            }))
        }
        Err(error) => Err(error),
    }
}

/// Whether a failed connect means "nothing is serving this endpoint".
///
/// `ConnectionRefused` is the Unix answer for a closed port: the kernel rejects
/// the SYN, so the endpoint is provably empty. Not every platform reports an
/// absent *loopback* listener that way. Windows reports the same empty port as a
/// connect timeout when the attempt is dropped rather than rejected, and a
/// loaded runner can also spend the whole connect budget before the refusal
/// arrives. CI measured exactly that on `127.0.0.1:59051` and `[::1]:59040`,
/// where nothing was listening:
///
/// ```text
/// Transport(ConnectError { details: "unable to connect to Future Agent
/// (http://127.0.0.1:59051: transport error)", kind: Some(TimedOut) })
/// ```
///
/// For an endpoint this host is allowed to serve, an attempt that never
/// established is the same fact as a refusal: there is no Agent there to attach
/// to. Two things keep that from being a wider reading of every timeout:
///
/// - Only a loopback TCP endpoint qualifies ([`PermitEndpoint::Tcp`] is already
///   validated loopback above), never the local IPC endpoint, whose socket
///   either exists or does not.
/// - A peer that *does* accept the connection but never completes readiness
///   fails the handshake (`ConnectionError::Unverified`), which never reaches
///   this function — so a busy or stalled Agent is not an absent one.
///
/// The permit is only permission to try: the Agent's own bind (a second binder
/// gets `AddrInUse`) and the instance lock remain the authority, exactly as they
/// are for a refused endpoint.
fn nothing_answers(error: &transport::ConnectError, endpoint: &PermitEndpoint) -> bool {
    match error.kind {
        Some(io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => true,
        Some(io::ErrorKind::TimedOut) => matches!(endpoint, PermitEndpoint::Tcp(_)),
        _ => false,
    }
}

#[cfg(test)]
pub(crate) mod tests;
