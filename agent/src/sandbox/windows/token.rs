//! Windows restricted-token primitives. Deliberately not connected to shell
//! spawning until the W2 AccessCheck matrix has run on Windows.

#![allow(dead_code)]

use std::collections::HashSet;
use std::io;
use std::ptr;

use sha2::{Digest, Sha256};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, HANDLE};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, IsTokenRestricted, DISABLE_MAX_PRIVILEGE, LUA_TOKEN, PSID,
    SID_AND_ATTRIBUTES, TOKEN_DUPLICATE, TOKEN_QUERY, WRITE_RESTRICTED,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::super::windows_capability::CapabilityRecord;

/// Owned, movable SID bytes. The backing allocation never moves when this
/// wrapper moves, so pointers handed to synchronous Win32 calls remain stable.
pub(crate) struct OwnedSid(Vec<u8>);

impl OwnedSid {
    pub(crate) fn as_psid(&self) -> PSID {
        self.0.as_ptr().cast_mut().cast()
    }
}

/// A primary WRITE_RESTRICTED token. It preserves the caller's normal SID
/// check, removes maximum privileges, applies LUA filtering, and adds only the
/// explicitly supplied capability SIDs to the second (write-only) check.
pub(crate) struct RestrictedToken(HANDLE);

unsafe impl Send for RestrictedToken {}
unsafe impl Sync for RestrictedToken {}

impl RestrictedToken {
    pub(crate) fn from_capabilities(records: &[CapabilityRecord]) -> io::Result<Self> {
        if records.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a write-restricted token requires at least one capability",
            ));
        }
        let mut names = HashSet::new();
        let sids: Vec<OwnedSid> = records
            .iter()
            .filter(|record| names.insert(record.name.as_str()))
            .map(|record| derive_capability_sid(&record.name))
            .collect::<io::Result<_>>()?;
        let restricted: Vec<SID_AND_ATTRIBUTES> = sids
            .iter()
            .map(|sid| SID_AND_ATTRIBUTES {
                Sid: sid.as_psid(),
                // Microsoft requires zero for SidsToRestrict; restricting SIDs
                // are always enabled for their access-check pass.
                Attributes: 0,
            })
            .collect();

        let mut source = ptr::null_mut();
        // SAFETY: pseudo process handle is valid; `source` is initialized on
        // success and closed on every path below.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY,
                &mut source,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let source = HandleGuard(source);
        let mut token = ptr::null_mut();
        // SAFETY: SID backing allocations and the array stay alive throughout
        // the synchronous call. Empty disable/privilege arrays are null.
        let ok = unsafe {
            CreateRestrictedToken(
                source.0,
                DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED,
                0,
                ptr::null(),
                0,
                ptr::null(),
                restricted.len() as u32,
                restricted.as_ptr(),
                &mut token,
            )
        };
        if ok == 0 {
            return Err(map_restricted_token_error(io::Error::last_os_error()));
        }
        let token = RestrictedToken(token);
        // IsTokenRestricted reports false when no restricting SID actually
        // made it into the token. Treat that as fail-closed initialization.
        if unsafe { IsTokenRestricted(token.0) } == 0 {
            return Err(io::Error::other(
                "CreateRestrictedToken returned a token without restricting SIDs",
            ));
        }
        Ok(token)
    }

    pub(crate) fn as_handle(&self) -> HANDLE {
        self.0
    }
}

fn map_restricted_token_error(error: io::Error) -> io::Error {
    // Some Windows hosts reject CreateRestrictedToken with 87 even for a
    // normal unelevated token and valid capability SIDs. Do not fall back to
    // an ordinary token: this is a feature-probe failure and the Windows
    // write-protection backend must remain unavailable.
    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows host rejected CreateRestrictedToken (ERROR_INVALID_PARAMETER); Windows write protection is unavailable on this host",
        )
    } else {
        error
    }
}

impl Drop for RestrictedToken {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Build a stable, syntactically-valid account-domain SID for one FutureOS
/// capability identity.
///
/// `DeriveCapabilitySidsFromName` returns an **AppContainer** capability SID.
/// That type belongs in AppContainer tokens and is rejected by
/// `CreateRestrictedToken` on normal desktop Windows (ERROR_INVALID_PARAMETER
/// on a clean Windows 11 Home install). A WRITE_RESTRICTED token instead needs
/// ordinary restricting SIDs. Like Codex's legacy Windows backend, we derive
/// a private, stable account-domain-shaped SID from the immutable capability
/// name. It need not correspond to a SAM account: it is only a DACL trustee
/// and restricting SID.
pub(crate) fn derive_capability_sid(name: &str) -> io::Result<OwnedSid> {
    if name.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capability name must not be empty",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"futureos-windows-restricting-sid-v1\0");
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    // Binary SID: revision, subauthority count, identifier authority
    // (big-endian), then subauthorities (little-endian): S-1-5-21-a-b-c-d.
    let mut bytes = Vec::with_capacity(28);
    bytes.extend_from_slice(&[1, 5, 0, 0, 0, 0, 0, 5]);
    bytes.extend_from_slice(&21u32.to_le_bytes());
    for chunk in digest[..16].chunks_exact(4) {
        bytes.extend_from_slice(chunk);
    }
    Ok(OwnedSid(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_parameter_is_a_fail_closed_host_probe_result() {
        let error = map_restricted_token_error(io::Error::from_raw_os_error(
            ERROR_INVALID_PARAMETER as i32,
        ));
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("CreateRestrictedToken"));
    }

    #[test]
    fn capability_sid_is_stable_and_uses_account_domain_layout() {
        let first = derive_capability_sid("futureos.windows.one").unwrap();
        let same = derive_capability_sid("futureos.windows.one").unwrap();
        let other = derive_capability_sid("futureos.windows.two").unwrap();
        assert_eq!(first.0, same.0);
        assert_ne!(first.0, other.0);
        assert_eq!(&first.0[..8], &[1, 5, 0, 0, 0, 0, 0, 5]);
        assert_eq!(u32::from_le_bytes(first.0[8..12].try_into().unwrap()), 21);
        assert_eq!(first.0.len(), 28);
    }
}
