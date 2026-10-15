//! Endpoint ownership is independent of the Agent data-directory lock. The
//! lease file is never unlinked: all cooperating binders lock the same inode.
use super::{LocalIncoming, LocalIo};
use std::{
    fs::File,
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::Duration,
};

pub struct LocalEndpointReservation {
    path: PathBuf,
    lease: File,
}

fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
fn conflict(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::AddrInUse, message.into())
}

impl LocalEndpointReservation {
    pub(super) async fn acquire(path: PathBuf) -> io::Result<Self> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "local socket has no parent directory",
            )
        })?;
        std::fs::create_dir_all(parent).map_err(|error| super::bind_dir_error(error, parent))?;
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
        let lease = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&lease_path)?;
        let metadata = lease.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "untrusted local endpoint lease",
            ));
        }
        if unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&lease),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } != 0
        {
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
        match tokio::time::timeout(
            Duration::from_millis(250),
            tokio::net::UnixStream::connect(&self.path),
        )
        .await
        {
            Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {
                Ok(Some(identity(&metadata)))
            }
            _ => Err(conflict(format!(
                "local endpoint is active or cannot be classified as stale: {}",
                self.path.display()
            ))),
        }
    }

    pub async fn bind(self) -> io::Result<LocalIncoming> {
        if let Some(stale) = self.stale_socket().await? {
            let current = std::fs::symlink_metadata(&self.path)?;
            if identity(&current) != stale {
                return Err(conflict("local endpoint changed before stale cleanup"));
            }
            std::fs::remove_file(&self.path)?;
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
    _lease: File,
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
    async fn endpoint_lease_is_exclusive_across_processes_even_before_binding() {
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
    async fn active_legacy_listener_is_preserved_but_stale_socket_is_reclaimed() {
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
