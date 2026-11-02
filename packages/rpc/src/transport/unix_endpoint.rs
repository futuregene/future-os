//! Endpoint ownership is independent of the Agent data-directory lock. The
//! lease file is never unlinked: all cooperating binders lock the same inode.
//!
//! Ownership ends when the owning process drops its lease, so the lease is
//! released explicitly (`LOCK_UN`) rather than left to `close()`: a `fork()`ed
//! child inherits the lease's *file description*, and `flock` locks belong to
//! the description, not to the descriptor. See [`LeaseFile`].
use super::{LocalIncoming, LocalIo};
use std::{
    fs::File,
    io,
    os::fd::{AsRawFd, RawFd},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::Duration,
};

/// The lease file, held open while this process owns the endpoint.
///
/// `flock` locks belong to the open *file description*: a `fork()`ed child
/// inherits a reference to the description, so closing our own descriptor does
/// not release the lock until the child `exec`s (which closes the inherited
/// descriptor, opened `O_CLOEXEC`). During that fork→exec window the endpoint
/// still reads as occupied to the next binder, and dropping the reservation is
/// expected to hand the endpoint over immediately — measured on this host:
/// ~70 µs when the child's image is warm, and hundreds of milliseconds when it
/// is cold and the host is loaded. Releasing with `LOCK_UN` ends ownership at
/// the drop, whatever the child still references (which is also what `fork(2)`
/// documents: "if a process holding a lock on a file forks and the child
/// explicitly unlocks the file, the parent will lose its lock").
struct LeaseFile(File);

impl LeaseFile {
    fn fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Take the exclusive, non-blocking lease. `false` means another binder
    /// owns the endpoint.
    fn try_lock(&self) -> bool {
        unsafe { libc::flock(self.fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    }
}

impl Drop for LeaseFile {
    fn drop(&mut self) {
        // Unlock before the descriptor is closed: `close()` alone is not a
        // release while another descriptor — a `dup`, or a `fork()`ed child that
        // has not exec'd yet — still references this file description.
        unsafe { libc::flock(self.fd(), libc::LOCK_UN) };
    }
}

pub struct LocalEndpointReservation {
    path: PathBuf,
    lease: LeaseFile,
}

fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
fn conflict(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::AddrInUse, message.into())
}

/// How long a connect probe may take before the endpoint is preserved as
/// unclassified. A listener that is merely slow to accept would otherwise be
/// replaced under it.
const CONNECT_PROBE: Duration = Duration::from_millis(250);

/// Name the outcome that preserved the endpoint, because the three cases are
/// three different defects: a listener answered (an active endpoint, correctly
/// preserved), a connect failed with a kind that is not a refusal, or the probe
/// never got to run before its own budget elapsed — a loaded host, which says
/// nothing about the endpoint. Reporting one string for all three is what made
/// the first measurement of the lease flake ambiguous.
fn probe_outcome(
    outcome: &Result<io::Result<tokio::net::UnixStream>, tokio::time::error::Elapsed>,
) -> String {
    match outcome {
        Ok(Ok(_)) => "a listener accepted the connection".to_string(),
        Ok(Err(error)) => format!("connect failed with {:?}: {error}", error.kind()),
        Err(_) => format!("the connect probe exceeded {CONNECT_PROBE:?}"),
    }
}

impl LocalEndpointReservation {
    pub(super) async fn acquire(path: PathBuf) -> io::Result<Self> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "local socket has no parent directory",
            )
        })?;
        // The short fallback endpoint lives in a world-writable directory, so its
        // directory is verified rather than created-and-chmodded: creating it with
        // `create_dir_all` and then fixing the mode would rewrite a directory
        // somebody else made, and on `/tmp` the chmod would be refused anyway.
        // An endpoint inside a FutureOS home needs none of this — that directory
        // is the instance's own, below.
        if parent == super::endpoint_dir() {
            super::ensure_endpoint_dir()?;
        } else {
            std::fs::create_dir_all(parent)
                .map_err(|error| super::bind_dir_error(error, parent))?;
        }
        let metadata =
            std::fs::metadata(parent).map_err(|error| super::bind_dir_error(error, parent))?;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "local socket directory is not owned by the current user",
            ));
        }
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        let mut lease_path = path.as_os_str().to_os_string();
        lease_path.push(".owner.lock");
        let lease_path = PathBuf::from(lease_path);
        let lease = LeaseFile(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&lease_path)?,
        );
        let metadata = lease.0.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "untrusted local endpoint lease",
            ));
        }
        if !lease.try_lock() {
            return Err(conflict(format!(
                "local endpoint ownership is occupied: {}",
                path.display()
            )));
        }
        if identity(&std::fs::symlink_metadata(&lease_path)?) != identity(&metadata) {
            return Err(conflict(
                "local endpoint lease changed while acquiring ownership",
            ));
        }
        let reservation = Self { path, lease };
        reservation.stale_socket().await?;
        Ok(reservation)
    }

    // Connection success, timeout, permissions and unknown errors all preserve
    // the endpoint. Only an owned socket refusing connections is stale.
    async fn stale_socket(&self) -> io::Result<Option<(u64, u64)>> {
        let metadata = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to replace untrusted local endpoint",
            ));
        }
        let outcome =
            tokio::time::timeout(CONNECT_PROBE, tokio::net::UnixStream::connect(&self.path)).await;
        match outcome {
            Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {
                Ok(Some(identity(&metadata)))
            }
            outcome => Err(conflict(format!(
                "local endpoint is active or cannot be classified as stale: {} ({})",
                self.path.display(),
                probe_outcome(&outcome)
            ))),
        }
    }

    /// Remove the socket [`Self::stale_socket`] classified as `stale`, unless the
    /// path no longer names that inode.
    ///
    /// The classification and the removal are two syscalls apart, and anything
    /// that bound a fresh listener in between owns the path now: replacing it
    /// would delete a live endpoint. This is the guard the campaign's
    /// "biased race" fixture used to search for with spinning churner threads;
    /// it is a decision about two identities, so it is driven directly.
    fn remove_stale_socket(&self, stale: (u64, u64)) -> io::Result<()> {
        let current = std::fs::symlink_metadata(&self.path)?;
        if identity(&current) != stale {
            return Err(conflict("local endpoint changed before stale cleanup"));
        }
        std::fs::remove_file(&self.path)
    }

    pub async fn bind(self) -> io::Result<LocalIncoming> {
        if let Some(stale) = self.stale_socket().await? {
            self.remove_stale_socket(stale)?;
        }
        let listener = tokio::net::UnixListener::bind(&self.path)?;
        let socket_identity = identity(&std::fs::symlink_metadata(&self.path)?);
        let cleanup = SocketCleanup {
            path: self.path,
            socket_identity,
            _lease: self.lease,
        };
        std::fs::set_permissions(&cleanup.path, std::fs::Permissions::from_mode(0o600))?;
        let uid = unsafe { libc::geteuid() };
        Ok(Box::pin(async_stream::stream! {
            let _cleanup = cleanup;
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => match stream.peer_cred() {
                        Ok(cred) if cred.uid() == uid => yield Ok(LocalIo { inner: stream }),
                        Ok(_) => {},
                        Err(error) => yield Err(error),
                    },
                    Err(error) => yield Err(error),
                }
            }
        }))
    }
}

struct SocketCleanup {
    path: PathBuf,
    socket_identity: (u64, u64),
    _lease: LeaseFile,
}
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| identity(&metadata) == self.socket_identity)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A descriptor duplicated from another one, closed on drop so a failed
    /// assertion cannot leak the endpoint's lease into the rest of the suite.
    struct SharedFd(RawFd);

    impl Drop for SharedFd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }

    /// The same open file description under a second descriptor — what a
    /// `fork()`ed child holds until it `exec`s, reproduced in-process. `flock`
    /// belongs to the description, so this is what makes `close()` a release
    /// only when nothing else references it.
    fn dup_of(fd: RawFd) -> SharedFd {
        let duplicate = unsafe { libc::dup(fd) };
        assert!(duplicate >= 0, "dup failed: {}", io::Error::last_os_error());
        SharedFd(duplicate)
    }

    /// Serializes the tests that `fork` with the tests whose assertion depends on
    /// a descriptor actually being released.
    ///
    /// `fork(2)` copies the *whole* descriptor table into the child, and until
    /// that child `exec`s — which is what closes the inherited, `O_CLOEXEC`
    /// descriptors — every descriptor another test holds is duplicated: dropping
    /// a listener is not a release while the window lasts. Measured on this host:
    /// ~70 µs when the child's image is warm, still holding after 300 ms when it
    /// is cold and the host is loaded. A test that spawns a process therefore
    /// holds this guard for the child's whole fork window, and so does the test
    /// that asserts a released listener is reclaimable. (The lease tests need no
    /// guard: releasing a lease is an explicit `LOCK_UN`, see [`LeaseFile`].)
    fn fork_guard() -> std::sync::MutexGuard<'static, ()> {
        static FORK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        FORK_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[test]
    fn lease_child_fixture() {
        let Some(path) = std::env::var_os("FUTURE_ENDPOINT_LEASE_FIXTURE") else {
            return;
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(LocalEndpointReservation::acquire(PathBuf::from(path)))
            .err()
            .expect("parent owns endpoint lease");
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    }

    #[tokio::test]
    // The fork guard is held across the awaits on purpose (no other test may
    // release a descriptor while this child's fork window is open) on a
    // current-thread runtime, so no task shares the thread with it.
    #[allow(clippy::await_holding_lock)]
    async fn endpoint_lease_is_exclusive_across_processes_even_before_binding() {
        let fork_window = fork_guard();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let _reserved = LocalEndpointReservation::acquire(path.clone())
            .await
            .unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "transport::unix_endpoint::tests::lease_child_fixture",
                "--nocapture",
            ])
            .env("FUTURE_ENDPOINT_LEASE_FIXTURE", &path)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("lease fixture timed out");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(fork_window);
    }

    #[tokio::test]
    async fn reservations_reject_symlink_leases_and_preserve_non_socket_occupants() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let target = root.path().join("target");
        std::fs::write(&target, "keep").unwrap();
        let lease = root.path().join("agent.sock.owner.lock");
        std::os::unix::fs::symlink(&target, &lease).unwrap();
        assert!(LocalEndpointReservation::acquire(path.clone())
            .await
            .is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        std::fs::remove_file(lease).unwrap();
        std::fs::write(&path, "occupant").unwrap();
        assert!(LocalEndpointReservation::acquire(path.clone())
            .await
            .is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "occupant");
    }

    /// The lease is opened without following symlinks and must be a private,
    /// singly-linked regular file of the current user. A second hard link is how
    /// another local user could keep a handle on this endpoint's lease, so it is
    /// refused before the lock is even attempted.
    #[tokio::test]
    async fn a_multiply_linked_lease_is_refused_as_untrusted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let lease = root.path().join("agent.sock.owner.lock");
        std::fs::write(&lease, "").unwrap();
        std::fs::hard_link(&lease, root.path().join("lease-alias")).unwrap();

        let error = LocalEndpointReservation::acquire(path)
            .await
            .err()
            .expect("a shared lease must not be trusted");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            error.to_string().contains("untrusted local endpoint lease"),
            "{error}"
        );
        // Nothing was created for the endpoint itself.
        assert!(!root.path().join("agent.sock").exists());
    }

    /// A path with no parent directory can never own a socket directory, and
    /// saying so is better than creating something at the filesystem root.
    #[tokio::test]
    async fn an_endpoint_without_a_parent_directory_is_refused() {
        for path in [PathBuf::from("/"), PathBuf::from("")] {
            let error = LocalEndpointReservation::acquire(path.clone())
                .await
                .err()
                .expect("a path with no parent cannot be reserved");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("no parent directory"), "{error}");
        }
    }

    /// Dropping a reservation must free the endpoint even when another
    /// descriptor still references the lease's file description — which is what a
    /// `fork()`ed child holds until it `exec`s, and what `dup` reproduces here.
    /// `flock` lives on the description, so without the explicit `LOCK_UN` in
    /// [`LeaseFile::drop`] the next binder reports `AddrInUse: local endpoint
    /// ownership is occupied` for as long as the window lasts: the shape (and the
    /// message) these tests used to flake with at ~5 % of runs.
    #[tokio::test]
    async fn dropping_a_reservation_releases_the_lease_a_shared_description_references() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let reserved = LocalEndpointReservation::acquire(path.clone())
            .await
            .unwrap();
        let shared = dup_of(reserved.lease.fd());

        drop(reserved);
        let listener = LocalEndpointReservation::acquire(path.clone())
            .await
            .expect("the owner's release must free the endpoint")
            .bind()
            .await
            .unwrap();
        drop(listener);
        drop(shared);
        assert!(!path.exists());
    }

    /// `stale_socket` classifies an endpoint by connecting to it, and a listener
    /// whose descriptor another descriptor still references (here a `dup`; in a
    /// fork window, a child) is not released by dropping the listener: the socket
    /// stays connectable. This is why [`fork_guard`] serializes that
    /// classification against the tests that spawn a process — a run that fails
    /// here means a released listener was really still reachable.
    #[tokio::test]
    // The fork guard is held across the awaits on purpose (the listener dropped
    // below must really be released) on a current-thread runtime, so no task
    // shares the thread with it.
    #[allow(clippy::await_holding_lock)]
    async fn a_duplicated_listener_descriptor_keeps_a_dropped_socket_connectable() {
        let _fork_window = fork_guard();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let shared = dup_of(listener.as_raw_fd());

        drop(listener);
        assert!(
            tokio::net::UnixStream::connect(&path).await.is_ok(),
            "the duplicated descriptor must keep the socket connectable"
        );
        drop(shared);
        assert_eq!(
            tokio::net::UnixStream::connect(&path)
                .await
                .err()
                .map(|error| error.kind()),
            Some(io::ErrorKind::ConnectionRefused),
            "with the last descriptor gone the socket must be refused"
        );
    }

    #[tokio::test]
    async fn lease_excludes_other_binders_and_allows_clean_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let reserved = LocalEndpointReservation::acquire(path.clone())
            .await
            .unwrap();
        assert!(LocalEndpointReservation::acquire(path.clone())
            .await
            .is_err());
        let listener = reserved.bind().await.unwrap();
        assert!(LocalEndpointReservation::acquire(path.clone())
            .await
            .is_err());
        drop(listener);
        drop(
            LocalEndpointReservation::acquire(path)
                .await
                .unwrap()
                .bind()
                .await
                .unwrap(),
        );
    }
    #[tokio::test]
    // The fork guard is held across the awaits on purpose (the listener must
    // really be released before the stale-socket classification below runs) on a
    // current-thread runtime, so no task shares the thread with it.
    #[allow(clippy::await_holding_lock)]
    async fn active_legacy_listener_is_preserved_but_stale_socket_is_reclaimed() {
        let _fork_window = fork_guard();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let before = identity(&std::fs::metadata(&path).unwrap());
        assert!(LocalEndpointReservation::acquire(path.clone())
            .await
            .is_err());
        assert_eq!(identity(&std::fs::metadata(&path).unwrap()), before);
        drop(listener);
        drop(
            LocalEndpointReservation::acquire(path.clone())
                .await
                .unwrap()
                .bind()
                .await
                .unwrap(),
        );
        assert!(!path.exists());
    }
    /// `bind` re-reads the socket's identity after the `stale_socket()` probe and
    /// refuses to remove a socket that changed in between (155-157).
    ///
    /// The window is the two syscalls around the probe, and the guard itself is a
    /// comparison of two identities — so it is driven, not hunted. This used to be
    /// the campaign's "biased race" fixture: four spinning churner threads per
    /// attempt for up to 100 000 attempts, waiting for one replacement to land in
    /// the window. That search made the test's outcome a property of the host
    /// rather than of the code — the fixture's own churners starve the
    /// `CONNECT_PROBE` budget (the drive then reports "cannot be classified as
    /// stale" from `acquire`, which has nothing to do with this guard) and the
    /// siblings that measure time. Here every run reaches the guard: classify, let
    /// another owner take the path, then ask for the removal.
    ///
    /// The `fork_guard` is load-bearing for the same reason it is in
    /// [`active_legacy_listener_is_preserved_but_stale_socket_is_reclaimed`]: a
    /// dropped listener is only refused once nothing references its file
    /// description, and a sibling test's `fork` window references every
    /// descriptor this process holds. Without the guard this test reds at that
    /// window with `acquire` reporting "a listener accepted the connection" —
    /// measured 2 of 48 runs under load 2026-11-02, which is the same arm (and
    /// the same rate) the churn-based version of this test used to flake with.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn a_socket_replaced_between_the_probe_and_the_cleanup_is_refused() {
        let _fork_window = fork_guard();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let scratch = root.path().join("replacement.sock");
        // A socket that was bound and closed is stale: connects are refused, so
        // `acquire` classifies it and hands the reservation over.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        let reservation = LocalEndpointReservation::acquire(path.clone())
            .await
            .unwrap();
        let stale = reservation
            .stale_socket()
            .await
            .unwrap()
            .expect("a closed socket must classify as stale");

        // The window: another owner binds a fresh listener and moves it onto the
        // path. Both inodes exist at once, so the replacement cannot alias the
        // classified one through inode reuse, and `rename` keeps the path present
        // (the drive cannot fall into the neighbouring `NotFound` case).
        let replacement = std::os::unix::net::UnixListener::bind(&scratch).unwrap();
        let owner = identity(&std::fs::symlink_metadata(&scratch).unwrap());
        assert_ne!(owner, stale, "the fixture must present a different socket");
        std::fs::rename(&scratch, &path).unwrap();

        let error = reservation
            .remove_stale_socket(stale)
            .expect_err("a socket replaced after the probe must be refused");
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(
            error.to_string().contains("changed before stale cleanup"),
            "{error}"
        );
        assert_eq!(
            identity(&std::fs::symlink_metadata(&path).unwrap()),
            owner,
            "the refusal must leave the replacement in place"
        );
        // And it is still serving, which is what the refusal is for.
        assert!(
            tokio::net::UnixStream::connect(&path).await.is_ok(),
            "the replacement listener must still accept after the refusal"
        );
        drop(replacement);
    }

    /// The other half of the same comparison: an identity that did not change is
    /// removed, so `bind` still reclaims the stale socket it classified. It needs
    /// the `fork_guard` for the reason spelled out above — "bound and dropped" is
    /// only stale while no sibling `fork` window references the descriptor.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn an_unchanged_stale_socket_is_removed_after_the_probe() {
        let _fork_window = fork_guard();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        let reservation = LocalEndpointReservation::acquire(path.clone())
            .await
            .unwrap();
        let stale = reservation
            .stale_socket()
            .await
            .unwrap()
            .expect("a closed socket must classify as stale");
        reservation.remove_stale_socket(stale).unwrap();
        assert!(!path.exists(), "the classified socket must be gone");
    }

    #[tokio::test]
    async fn old_cleanup_cannot_delete_a_replacement_socket() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.sock");
        let old = LocalEndpointReservation::acquire(path.clone())
            .await
            .unwrap()
            .bind()
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(old);
        assert!(path.exists());
        drop(replacement);
    }
}
