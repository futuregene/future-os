//! Additive ACL mutation for FutureOS-owned capability SIDs.

#![allow(dead_code)]

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
use windows_sys::Win32::Security::Authorization::{
    GetExplicitEntriesFromAclW, GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, DENY_ACCESS,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    EqualSid, ACL, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, OBJECT_INHERIT_ACE, PSID,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_GENERIC_WRITE};

use super::audit::FrozenPath;
use super::token::OwnedSid;

const CHILDREN_ONLY: u32 =
    OBJECT_INHERIT_ACE | SUB_CONTAINERS_AND_OBJECTS_INHERIT | INHERIT_ONLY_ACE;

/// Ensure write access for the capability SID on a root without granting
/// WRITE_DAC, WRITE_OWNER, or FILE_DELETE_CHILD. DELETE is inherited by child
/// objects only, so a capability cannot delete the approved root itself.
pub(crate) fn ensure_write_root(target: &FrozenPath, sid: &OwnedSid) -> io::Result<()> {
    ensure_entry(
        target,
        sid,
        FILE_GENERIC_WRITE,
        GRANT_ACCESS,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    )?;
    ensure_entry(target, sid, DELETE, GRANT_ACCESS, CHILDREN_ONLY)
}

/// Ensure content-write access on one existing file. No inheritance and no
/// DELETE right: `scope=file` must not become directory/subtree authority.
pub(crate) fn ensure_write_file(target: &FrozenPath, sid: &OwnedSid) -> io::Result<()> {
    ensure_entry(target, sid, FILE_GENERIC_WRITE, GRANT_ACCESS, 0)
}

/// Deny all ordinary write rights and deletion on an ask/deny carveout. Since
/// no parent capability receives FILE_DELETE_CHILD, this object-level deny
/// cannot be bypassed via the writable parent directory.
pub(crate) fn ensure_write_deny(target: &FrozenPath, sid: &OwnedSid) -> io::Result<()> {
    ensure_entry(
        target,
        sid,
        FILE_GENERIC_WRITE | DELETE,
        DENY_ACCESS,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    )
}

fn ensure_entry(
    target: &FrozenPath,
    sid: &OwnedSid,
    permissions: u32,
    mode: i32,
    inheritance: u32,
) -> io::Result<()> {
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            target.handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalGuard(descriptor);
    if entry_present(dacl, sid.as_psid(), permissions, mode, inheritance)? {
        return Ok(());
    }

    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: permissions,
        grfAccessMode: mode,
        grfInheritance: inheritance,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.as_psid().cast(),
        },
    };
    let mut updated: *mut ACL = ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(1, &entry, dacl, &mut updated) };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let updated = LocalGuard(updated.cast());
    let status = unsafe {
        SetSecurityInfo(
            target.handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            updated.0.cast(),
            ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    drop(descriptor);
    Ok(())
}

fn entry_present(
    dacl: *const ACL,
    sid: PSID,
    permissions: u32,
    mode: i32,
    inheritance: u32,
) -> io::Result<bool> {
    if dacl.is_null() {
        return Ok(false);
    }
    let mut count = 0;
    let mut entries: *mut EXPLICIT_ACCESS_W = ptr::null_mut();
    let status = unsafe { GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let entries_guard = LocalGuard(entries.cast());
    let found = unsafe { std::slice::from_raw_parts(entries, count as usize) }
        .iter()
        .any(|entry| {
            entry.grfAccessMode == mode
                && entry.grfAccessPermissions & permissions == permissions
                && entry.grfInheritance & inheritance == inheritance
                && entry.Trustee.TrusteeForm == TRUSTEE_IS_SID
                && !entry.Trustee.ptstrName.is_null()
                && unsafe { EqualSid(entry.Trustee.ptstrName.cast(), sid) } != 0
        });
    drop(entries_guard);
    Ok(found)
}

struct LocalGuard(*mut core::ffi::c_void);

impl Drop for LocalGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0) };
        }
    }
}
