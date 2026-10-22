//! DEB installs use one system authorization and a prepared native executor.
//! Package-manager failure is never inferred to be rollback from file hashes.
use super::{
    journal,
    process::{self, Completion, PreparedProcess},
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, process::Command};

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn field(path: &Path, name: &str) -> Result<String, String> {
    output(
        "/usr/bin/dpkg-deb",
        &[
            "--field",
            path.to_str().ok_or("Non-UTF8 package path")?,
            name,
        ],
    )
}
fn package_status(package: &str) -> Result<String, String> {
    output(
        "/usr/bin/dpkg-query",
        &[
            "--show",
            "--showformat=${db:Status-Status} ${Version}",
            package,
        ],
    )
}
fn hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(super) struct Prepared {
    helper: PreparedProcess,
    _package: tempfile::TempDir,
}
impl Prepared {
    pub(super) fn new(
        bytes: &[u8],
        signature: &str,
        operation: &journal::Operation,
    ) -> Result<Self, String> {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        let path = directory.path().join("update.deb");
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let owner = output(
            "/usr/bin/dpkg-query",
            &[
                "--search",
                executable.to_str().ok_or("Non-UTF8 executable path")?,
            ],
        )?;
        let owner = owner
            .rsplit_once(": ")
            .ok_or("The current executable is not owned by a DEB package")?
            .0;
        let package = field(&path, "Package")?;
        if owner.split(':').next() != Some(package.as_str()) {
            return Err("Update package does not match the installed application".into());
        }
        let version = field(&path, "Version")?;
        let plan = serde_json::json!({"package": package, "version": version});
        crate::config_io::write_json_atomic(
            &operation.path().with_file_name("linux.json"),
            &plan,
            true,
        )
        .map_err(|e| e.to_string())?;
        let mut command = Command::new("/usr/bin/pkexec");
        command
            .arg("--disable-internal-agent")
            .arg(&executable)
            .arg("--app-update-deb-worker")
            .arg(&path)
            .arg(hash(&path)?)
            .arg(signature)
            .arg(&package)
            .arg(&version)
            .arg(operation.path());
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(operation.path().with_file_name("worker.log"))
            .map_err(|e| e.to_string())?;
        command.stderr(log);
        let helper = PreparedProcess::start(command)?;
        Ok(Self {
            helper,
            _package: directory,
        })
    }
    pub(super) fn commit(self) -> Result<Completion, String> {
        self.helper.commit()
    }
}

pub(super) fn worker(args: &[String]) -> Result<(), String> {
    let [source, expected_hash, signature, package, version, record] = args else {
        return Err("Invalid DEB update worker arguments".into());
    };
    let uid: libc::uid_t = std::env::var("PKEXEC_UID")
        .map_err(|_| "A system authorization is required")?
        .parse()
        .map_err(|_| "Invalid authorization identity")?;
    if uid == 0 || unsafe { libc::geteuid() } != 0 {
        return Err("Invalid privileged update executor".into());
    }
    // Work only on a private root-owned copy. The unprivileged process cannot
    // change the archive after verification or while dpkg reads it.
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join("update.deb");
    std::fs::copy(source, &path).map_err(|e| e.to_string())?;
    if hash(&path)? != *expected_hash {
        return Err("Prepared update package changed".into());
    }
    use base64::Engine;
    let decode = |value: &str| -> Result<String, String> {
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    };
    let key = option_env!("FUTURE_UPDATER_PUBLIC_KEY").unwrap_or_default();
    if key.is_empty() {
        return Err("This build has no trusted update signing key".into());
    }
    let key = minisign_verify::PublicKey::decode(&decode(key)?).map_err(|e| e.to_string())?;
    let signature =
        minisign_verify::Signature::decode(&decode(signature)?).map_err(|e| e.to_string())?;
    key.verify(
        &std::fs::read(&path).map_err(|e| e.to_string())?,
        &signature,
        true,
    )
    .map_err(|e| format!("Privileged update signature verification failed: {e}"))?;
    if field(&path, "Package")? != *package || field(&path, "Version")? != *version {
        return Err("Signed update package identity does not match the transaction".into());
    }
    let dry_run = Command::new("/usr/bin/dpkg")
        .args(["--no-act", "--install"])
        .arg(&path)
        .output()
        .map_err(|e| e.to_string())?;
    if !dry_run.status.success() {
        return Err(format!(
            "Package preparation failed: {}",
            String::from_utf8_lossy(&dry_run.stderr)
        ));
    }
    // Never access a caller-controlled record or lock pathname as root.
    let passwd = unsafe { libc::getpwuid(uid) };
    if passwd.is_null() {
        return Err("Authorized account no longer exists".into());
    }
    let gid = unsafe { (*passwd).pw_gid };
    if unsafe { libc::setgroups(0, std::ptr::null()) } != 0
        || unsafe { libc::setegid(gid) } != 0
        || unsafe { libc::seteuid(uid) } != 0
    {
        return Err("Cannot isolate update journal access".into());
    }
    let record = Path::new(record);
    let _lease = journal::observer_lease(record).map_err(|e| e.to_string())?;
    if !process::await_commit()? {
        if unsafe { libc::seteuid(0) } == 0 {
            let _ = directory.close();
        }
        return Ok(());
    }
    let agent_lease = process::agent_lease(record);
    if unsafe { libc::seteuid(0) } != 0 || unsafe { libc::setegid(0) } != 0 {
        return Err("Cannot activate authorized installer".into());
    }
    let installed = if agent_lease.is_ok() {
        Some(
            Command::new("/usr/bin/dpkg")
                .arg("--install")
                .arg(&path)
                .output(),
        )
    } else {
        None
    };
    let result = match installed {
        None => Completion {
            ok: false,
            restored: true,
            diagnostic: agent_lease.as_ref().err().cloned().unwrap_or_default(),
        },
        Some(Ok(output))
            if output.status.success()
                && package_status(package).as_deref() == Ok(&format!("installed {version}")) =>
        {
            Completion::success()
        }
        Some(Ok(output)) => Completion {
            ok: false,
            restored: false,
            diagnostic: format!(
                "Package installation requires repair: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        },
        Some(Err(error)) => Completion {
            ok: false,
            restored: false,
            diagnostic: error.to_string(),
        },
    };
    // Remove the privileged staging area before permanently dropping identity.
    directory.close().map_err(|e| e.to_string())?;
    if unsafe { libc::setgid(gid) } != 0 || unsafe { libc::setuid(uid) } != 0 {
        return Err("Cannot release installer privileges".into());
    }
    journal::write_worker_result(
        record,
        if result.ok {
            "pending-validation"
        } else if result.restored {
            "failed-restored"
        } else {
            "repair-required"
        },
        (!result.ok).then(|| result.diagnostic.clone()),
        result.ok || result.restored,
    )
    .map_err(|e| e.to_string())?;
    process::complete(&result)
}
pub(super) fn confirm(id: &str) -> Result<(), String> {
    let plan = future_rpc::home::future_home()
        .join("app/updates")
        .join(id)
        .join("linux.json");
    let plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(plan).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let package = plan["package"]
        .as_str()
        .ok_or("Missing installed package identity")?;
    let version = plan["version"]
        .as_str()
        .ok_or("Missing installed package version")?;
    if package_status(package)? != format!("installed {version}") {
        return Err("New application started but DEB package configuration is incomplete".into());
    }
    Ok(())
}
