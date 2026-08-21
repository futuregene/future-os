//! Windows restricted-token primitives. Deliberately not connected to shell
//! spawning until the W2 AccessCheck matrix has run on Windows.

#![allow(dead_code)]

use std::collections::HashSet;
use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
use windows_sys::Win32::Security::{
    CopySid, CreateRestrictedToken, DeriveCapabilitySidsFromName, GetLengthSid, IsTokenRestricted,
    IsValidSid, DISABLE_MAX_PRIVILEGE, LUA_TOKEN, PSID, SID_AND_ATTRIBUTES, TOKEN_DUPLICATE,
    TOKEN_QUERY, WRITE_RESTRICTED,
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
            return Err(io::Error::last_os_error());
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

/// Derive the documented AppAuthority capability SID and copy it into Rust-
/// owned bytes before releasing every LocalAlloc allocation returned by Win32.
pub(crate) fn derive_capability_sid(name: &str) -> io::Result<OwnedSid> {
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut group_sids: *mut PSID = ptr::null_mut();
    let mut group_count = 0;
    let mut capability_sids: *mut PSID = ptr::null_mut();
    let mut capability_count = 0;
    // SAFETY: all output pointers are valid and initialized; Win32 allocates
    // both arrays and their SID elements on success.
    let ok = unsafe {
        DeriveCapabilitySidsFromName(
            wide.as_ptr(),
            &mut group_sids,
            &mut group_count,
            &mut capability_sids,
            &mut capability_count,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    let result = if capability_count == 1 && !capability_sids.is_null() {
        // SAFETY: successful API contract gives `capability_count` entries.
        let sid = unsafe { *capability_sids };
        clone_sid(sid)
    } else {
        Err(io::Error::other(format!(
            "expected one capability SID, got {capability_count}"
        )))
    };
    // SAFETY: Microsoft documents LocalFree for every SID and both arrays.
    unsafe {
        free_sid_array(group_sids, group_count);
        free_sid_array(capability_sids, capability_count);
    }
    result
}

fn clone_sid(sid: PSID) -> io::Result<OwnedSid> {
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Win32 returned an invalid capability SID",
        ));
    }
    let length = unsafe { GetLengthSid(sid) };
    let mut bytes = vec![0u8; length as usize];
    if unsafe { CopySid(length, bytes.as_mut_ptr().cast(), sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedSid(bytes))
}

unsafe fn free_sid_array(array: *mut PSID, count: u32) {
    if array.is_null() {
        return;
    }
    for sid in unsafe { std::slice::from_raw_parts(array, count as usize) } {
        if !sid.is_null() {
            unsafe { LocalFree(*sid) };
        }
    }
    unsafe { LocalFree(array.cast()) };
}
