//! Additive ACL mutation for FutureOS-owned capability SIDs.

#![allow(dead_code)]

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
use windows_sys::Win32::Security::Authorization::{
    GetExplicitEntriesFromAclW, GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, DENY_ACCESS,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, REVOKE_ACCESS, SE_FILE_OBJECT,
    TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
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

/// Remove every explicit ACE for one FutureOS-owned capability SID from an
/// object. Capability SIDs are unique to FutureOS, so `REVOKE_ACCESS` cannot
/// remove an unrelated user's entry. Inherited ACEs disappear when the root
/// entry is revoked; separately hardened carveouts are revoked explicitly.
pub(crate) fn revoke_capability(target: &FrozenPath, sid: &OwnedSid) -> io::Result<()> {
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
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: 0,
        grfAccessMode: REVOKE_ACCESS,
        // Match Codex's revoke_ace behavior. SetEntriesInAclW removes the
        // trustee's explicit root ACE; inherited child ACEs then disappear as
        // the parent ACL propagates.
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
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

#[cfg(test)]
pub(crate) fn capability_entry_present(target: &FrozenPath, sid: &OwnedSid) -> io::Result<bool> {
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
    let mut count = 0;
    let mut entries: *mut EXPLICIT_ACCESS_W = ptr::null_mut();
    let status = unsafe { GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let entries = LocalGuard(entries.cast());
    let found = explicit_entries_include_sid(entries.0.cast(), count, sid.as_psid())?;
    drop(descriptor);
    Ok(found)
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
    let found = explicit_entries_contain(entries, count, sid, permissions, mode, inheritance)?;
    drop(entries_guard);
    Ok(found)
}

fn explicit_entries_contain(
    entries: *const EXPLICIT_ACCESS_W,
    count: u32,
    sid: PSID,
    permissions: u32,
    mode: i32,
    inheritance: u32,
) -> io::Result<bool> {
    // GetExplicitEntriesFromAclW legitimately returns count=0 with a null list
    // when the ACL has no explicit entries. Rust slices require a non-null,
    // aligned pointer even for length zero, so handle that Win32 representation
    // before constructing a slice.
    if count == 0 {
        return Ok(false);
    }
    if entries.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Win32 returned a null explicit ACL list with a non-zero count",
        ));
    }
    // SAFETY: a successful GetExplicitEntriesFromAclW call owns `count`
    // initialized entries until the caller releases the enclosing LocalGuard.
    Ok(
        unsafe { std::slice::from_raw_parts(entries, count as usize) }
            .iter()
            .any(|entry| {
                entry.grfAccessMode == mode
                    && entry.grfAccessPermissions & permissions == permissions
                    && entry.grfInheritance & inheritance == inheritance
                    && entry.Trustee.TrusteeForm == TRUSTEE_IS_SID
                    && !entry.Trustee.ptstrName.is_null()
                    && unsafe { EqualSid(entry.Trustee.ptstrName.cast(), sid) } != 0
            }),
    )
}

fn explicit_entries_include_sid(
    entries: *const EXPLICIT_ACCESS_W,
    count: u32,
    sid: PSID,
) -> io::Result<bool> {
    if count == 0 {
        return Ok(false);
    }
    if entries.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Win32 returned a null explicit ACL list with a non-zero count",
        ));
    }
    Ok(
        unsafe { std::slice::from_raw_parts(entries, count as usize) }
            .iter()
            .any(|entry| {
                entry.Trustee.TrusteeForm == TRUSTEE_IS_SID
                    && !entry.Trustee.ptstrName.is_null()
                    && unsafe { EqualSid(entry.Trustee.ptstrName.cast(), sid) } != 0
            }),
    )
}

struct LocalGuard(*mut core::ffi::c_void);

impl Drop for LocalGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_explicit_acl_list_does_not_construct_a_null_slice() {
        assert!(
            !explicit_entries_contain(ptr::null(), 0, ptr::null_mut(), 0, GRANT_ACCESS, 0,)
                .unwrap()
        );
    }

    #[test]
    fn null_explicit_acl_list_with_entries_is_rejected() {
        let error = explicit_entries_contain(ptr::null(), 1, ptr::null_mut(), 0, GRANT_ACCESS, 0)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn null_sid_scan_with_entries_is_rejected() {
        let error = explicit_entries_include_sid(ptr::null(), 1, ptr::null_mut()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
