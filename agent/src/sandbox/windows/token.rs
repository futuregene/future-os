//! Windows restricted-token primitives used by the production sandbox runner.

use std::collections::HashSet;
use std::io;
use std::ptr;

use sha2::{Digest, Sha256};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_INVALID_PARAMETER, GENERIC_ALL, HANDLE, LUID,
};
use windows_sys::Win32::Security::Authorization::{
    SetEntriesInAclW, EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, CopySid, CreateRestrictedToken, CreateWellKnownSid, GetLengthSid,
    GetTokenInformation, IsTokenRestricted, IsValidSid, LookupPrivilegeValueW, SetTokenInformation,
    TokenDefaultDacl, TokenGroups, TokenUser, WinWorldSid, DISABLE_MAX_PRIVILEGE, LUA_TOKEN,
    LUID_AND_ATTRIBUTES, PSID, SE_CHANGE_NOTIFY_NAME, SE_PRIVILEGE_ENABLED, SID_AND_ATTRIBUTES,
    TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES, TOKEN_ASSIGN_PRIMARY, TOKEN_DEFAULT_DACL,
    TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER, WRITE_RESTRICTED,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::super::windows_capability::CapabilityRecord;

const SE_GROUP_LOGON_ID: u32 = 0xC0000000;

/// Owned, movable SID bytes. The backing allocation never moves when this
/// wrapper moves, so pointers handed to synchronous Win32 calls remain stable.
pub(crate) struct OwnedSid(Vec<u8>);

impl OwnedSid {
    pub(crate) fn as_psid(&self) -> PSID {
        self.0.as_ptr().cast_mut().cast()
    }
}

/// A primary WRITE_RESTRICTED token. It preserves the caller's normal SID
/// check, removes maximum privileges, applies LUA filtering, and adds the
/// explicitly supplied capability SIDs plus narrowly documented compatibility
/// identities to the second (write-only) check.
pub(crate) struct RestrictedToken {
    handle: HANDLE,
    // Keep every trustee passed to CreateRestrictedToken alive for the token's
    // lifetime. Capability identities are always first in this vector.
    sids: Vec<OwnedSid>,
    capability_sid_count: usize,
    // The private desktop must also pass the token's ordinary SID access check;
    // this identity is never added to the restricting SID set.
    normal_user_sid: OwnedSid,
}

unsafe impl Send for RestrictedToken {}
unsafe impl Sync for RestrictedToken {}

impl RestrictedToken {
    pub(crate) fn from_capabilities(records: &[CapabilityRecord]) -> io::Result<Self> {
        Self::from_capabilities_inner(records, true, true)
    }

    #[cfg(test)]
    pub(crate) fn from_capabilities_for_test(
        records: &[CapabilityRecord],
        include_logon: bool,
        include_everyone: bool,
    ) -> io::Result<Self> {
        Self::from_capabilities_inner(records, include_logon, include_everyone)
    }

    fn from_capabilities_inner(
        records: &[CapabilityRecord],
        include_logon: bool,
        include_everyone: bool,
    ) -> io::Result<Self> {
        if records.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a write-restricted token requires at least one capability",
            ));
        }

        let mut source = ptr::null_mut();
        // SAFETY: pseudo process handle is valid; `source` is initialized on
        // success and closed on every path below.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ASSIGN_PRIMARY
                    | TOKEN_ADJUST_DEFAULT
                    | TOKEN_ADJUST_PRIVILEGES
                    | TOKEN_DUPLICATE
                    | TOKEN_QUERY,
                &mut source,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let source = HandleGuard(source);
        let normal_user_sid = current_user_sid(source.0)?;

        let mut names = HashSet::new();
        let mut sids: Vec<OwnedSid> = records
            .iter()
            .filter(|record| names.insert(record.name.as_str()))
            .map(|record| derive_capability_sid(&record.name))
            .collect::<io::Result<_>>()?;
        // Codex's legacy unelevated backend adds the logon SID and Everyone to
        // the restricting SID set. PowerShell/CLR performs writes against
        // session-scoped and Everyone-accessible kernel objects while it boots;
        // without these two trustees those writes fail the WRITE_RESTRICTED
        // check and PowerShell exits with E_ACCESSDENIED before running any
        // command. These identities are broader than capabilities and can
        // match existing file ACLs, so the manual matrix must determine
        // whether either can be removed before release.
        let capability_sid_count = sids.len();
        if include_logon {
            sids.push(logon_sid(source.0)?);
        }
        if include_everyone {
            sids.push(everyone_sid()?);
        }

        let restricted: Vec<SID_AND_ATTRIBUTES> = sids
            .iter()
            .map(|sid| SID_AND_ATTRIBUTES {
                Sid: sid.as_psid(),
                // Microsoft requires zero for SidsToRestrict; restricting SIDs
                // are always enabled for their access-check pass.
                Attributes: 0,
            })
            .collect();

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
        let token = RestrictedToken {
            handle: token,
            sids,
            capability_sid_count,
            normal_user_sid,
        };
        // IsTokenRestricted reports false when no restricting SID actually
        // made it into the token. Treat that as fail-closed initialization.
        if unsafe { IsTokenRestricted(token.handle) } == 0 {
            return Err(io::Error::other(
                "CreateRestrictedToken returned a token without restricting SIDs",
            ));
        }
        set_default_dacl(&token)?;
        // DISABLE_MAX_PRIVILEGE disables this normal-user privilege. Restoring
        // it permits directory traversal without granting read/write access;
        // without it CreateProcessAsUserW cannot reach ordinary executable
        // paths and returns ERROR_ACCESS_DENIED.
        enable_change_notify_privilege(token.handle)?;
        Ok(token)
    }

    pub(crate) fn as_handle(&self) -> HANDLE {
        self.handle
    }

    /// Capability SIDs are safe trustees for FutureOS-created objects. Do not
    /// use the broader PowerShell compatibility identities for object DACLs.
    pub(crate) fn capability_sids(&self) -> &[OwnedSid] {
        &self.sids[..self.capability_sid_count]
    }

    pub(crate) fn normal_user_sid(&self) -> &OwnedSid {
        &self.normal_user_sid
    }
}

fn set_default_dacl(token: &RestrictedToken) -> io::Result<()> {
    // Named kernel objects created without an explicit descriptor inherit this
    // DACL. Include one ordinary identity and every capability identity so
    // both WRITE_RESTRICTED access-check passes can reopen the child's own
    // mutexes/events/pipes. Logon and Everyone exist only for compatibility in
    // the restricting set; granting them GENERIC_ALL here would unnecessarily
    // expose every child-created named object to other processes.
    let entries = std::iter::once(token.normal_user_sid())
        .chain(token.capability_sids())
        .map(|sid| EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid.as_psid().cast(),
            },
        })
        .collect::<Vec<_>>();
    let mut dacl = ptr::null_mut();
    let status = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            ptr::null(),
            &mut dacl,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let dacl = LocalAcl(dacl);
    let mut info = TOKEN_DEFAULT_DACL {
        DefaultDacl: dacl.0,
    };
    if unsafe {
        SetTokenInformation(
            token.handle,
            TokenDefaultDacl,
            ptr::addr_of_mut!(info).cast(),
            std::mem::size_of::<TOKEN_DEFAULT_DACL>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct LocalAcl(*mut windows_sys::Win32::Security::ACL);

impl Drop for LocalAcl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0.cast()) };
        }
    }
}

fn current_user_sid(token: HANDLE) -> io::Result<OwnedSid> {
    let mut bytes = 0;
    // The probe is expected to report required storage through `bytes`.
    unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes) };
    if bytes == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut storage = vec![0u8; bytes as usize];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            storage.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful TokenUser query initializes a TOKEN_USER at the
    // beginning of `storage`; clone_sid copies its pointed-to SID immediately.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    clone_sid(user.User.Sid)
}

fn logon_sid(token: HANDLE) -> io::Result<OwnedSid> {
    let mut bytes = 0;
    unsafe { GetTokenInformation(token, TokenGroups, ptr::null_mut(), 0, &mut bytes) };
    if bytes == 0 {
        return Err(io::Error::last_os_error());
    }
    // TOKEN_GROUPS contains pointer-aligned SID_AND_ATTRIBUTES entries. A
    // Vec<u8> does not promise that alignment, so use machine words as backing
    // storage and still pass the exact byte capacity to Win32.
    let storage_bytes = bytes as usize;
    let word_size = std::mem::size_of::<usize>();
    let word_count = storage_bytes.div_ceil(word_size);
    let mut storage = vec![0usize; word_count];
    if unsafe {
        GetTokenInformation(
            token,
            TokenGroups,
            storage.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // TOKEN_GROUPS is a DWORD count followed by a variable-length array. Check
    // the returned byte count before walking that tail; the kernel is trusted,
    // but keeping the unsafe boundary explicit prevents accidental UB if this
    // code is later reused with synthetic buffers.
    let returned_bytes = bytes as usize;
    if returned_bytes < std::mem::size_of::<u32>() || returned_bytes > storage_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid TokenGroups buffer length",
        ));
    }
    let count = unsafe { *storage.as_ptr().cast::<u32>() } as usize;
    let groups_offset = (std::mem::size_of::<u32>() + std::mem::align_of::<SID_AND_ATTRIBUTES>()
        - 1)
        & !(std::mem::align_of::<SID_AND_ATTRIBUTES>() - 1);
    let entries_bytes = count
        .checked_mul(std::mem::size_of::<SID_AND_ATTRIBUTES>())
        .and_then(|size| groups_offset.checked_add(size))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid TokenGroups count"))?;
    if entries_bytes > returned_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TokenGroups entries exceed returned buffer",
        ));
    }
    let groups = unsafe {
        storage
            .as_ptr()
            .cast::<u8>()
            .add(groups_offset)
            .cast::<SID_AND_ATTRIBUTES>()
    };
    for index in 0..count {
        let entry = unsafe { std::ptr::read_unaligned(groups.add(index)) };
        if entry.Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID {
            return clone_sid(entry.Sid);
        }
    }
    Err(io::Error::other("token has no logon SID"))
}

fn everyone_sid() -> io::Result<OwnedSid> {
    let mut bytes = 0;
    // The sizing probe is expected to report required storage through `bytes`.
    unsafe { CreateWellKnownSid(WinWorldSid, ptr::null_mut(), ptr::null_mut(), &mut bytes) };
    if bytes == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut storage = vec![0u8; bytes as usize];
    if unsafe {
        CreateWellKnownSid(
            WinWorldSid,
            ptr::null_mut(),
            storage.as_mut_ptr().cast(),
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedSid(storage))
}

fn clone_sid(sid: PSID) -> io::Result<OwnedSid> {
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Windows SID",
        ));
    }
    let length = unsafe { GetLengthSid(sid) };
    let mut bytes = vec![0u8; length as usize];
    if unsafe { CopySid(length, bytes.as_mut_ptr().cast(), sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedSid(bytes))
}

fn enable_change_notify_privilege(token: HANDLE) -> io::Result<()> {
    let mut luid = LUID {
        LowPart: 0,
        HighPart: 0,
    };
    if unsafe { LookupPrivilegeValueW(ptr::null(), SE_CHANGE_NOTIFY_NAME, &mut luid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    // AdjustTokenPrivileges may return success while reporting a final error,
    // so check GetLastError as required by the Win32 contract.
    unsafe { windows_sys::Win32::Foundation::SetLastError(0) };
    if unsafe { AdjustTokenPrivileges(token, 0, &privileges, 0, ptr::null_mut(), ptr::null_mut()) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let error = unsafe { GetLastError() };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(())
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
        unsafe { CloseHandle(self.handle) };
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
