//! Handle-based path freezing and local-NTFS validation.

use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
#[cfg(test)]
use windows_sys::Win32::Foundation::{GetLastError, LocalFree, ERROR_INSUFFICIENT_BUFFER};
#[cfg(test)]
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
#[cfg(test)]
use windows_sys::Win32::Security::{
    AccessCheck, DuplicateTokenEx, MakeAbsoluteSD, SecurityImpersonation, TokenImpersonation,
    DACL_SECURITY_INFORMATION, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PRIVILEGE_SET, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
    GetVolumeInformationByHandleW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};
#[cfg(test)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
};

#[cfg(test)]
use super::token::RestrictedToken;

/// An existing, non-reparse object frozen by handle. ACL operations use this
/// handle directly, eliminating name-based replacement between validation and
/// mutation.
pub(crate) struct FrozenPath {
    handle: HANDLE,
    final_path: PathBuf,
}

unsafe impl Send for FrozenPath {}
unsafe impl Sync for FrozenPath {}

impl FrozenPath {
    pub(crate) fn open_local_ntfs(path: &Path) -> io::Result<Self> {
        reject_non_local_path(path)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // WRITE_DAC is requested up front: if the current unelevated user cannot
        // change this DACL, approval must fail rather than trigger UAC or owner
        // changes later.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut frozen = Self {
            handle,
            final_path: PathBuf::new(),
        };
        frozen.reject_reparse()?;
        frozen.require_ntfs()?;
        frozen.final_path = frozen.query_final_path()?;
        reject_non_local_path(&frozen.final_path)?;
        if !same_windows_path(path, &frozen.final_path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "capability target changed during final-path validation: {} -> {}",
                    path.display(),
                    frozen.final_path.display()
                ),
            ));
        }
        Ok(frozen)
    }

    pub(crate) fn handle(&self) -> HANDLE {
        self.handle
    }

    /// Evaluate this object's current security descriptor against the exact
    /// restricted token for the native access-check matrix.
    #[cfg(test)]
    pub(crate) fn access_check(
        &self,
        token: &RestrictedToken,
        desired_access: u32,
    ) -> io::Result<bool> {
        let mut impersonation = std::ptr::null_mut();
        let ok = unsafe {
            DuplicateTokenEx(
                token.as_handle(),
                TOKEN_QUERY,
                std::ptr::null(),
                SecurityImpersonation,
                TokenImpersonation,
                &mut impersonation,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let impersonation = HandleGuard(impersonation);

        let mut descriptor = std::ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                self.handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let descriptor = LocalGuard(descriptor);
        let absolute = AbsoluteSecurityDescriptor::from_self_relative(descriptor.0)?;
        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ,
            GenericWrite: FILE_GENERIC_WRITE,
            GenericExecute: FILE_GENERIC_EXECUTE,
            GenericAll: FILE_ALL_ACCESS,
        };
        // PRIVILEGE_SET is variable-length. Keep aligned spare capacity; file
        // checks normally return an empty privilege set.
        let mut privilege_words = vec![0usize; 128];
        let mut privilege_length = (privilege_words.len() * std::mem::size_of::<usize>()) as u32;
        let mut granted = 0;
        let mut allowed = 0;
        let ok = unsafe {
            AccessCheck(
                absolute.descriptor.as_ptr().cast_mut().cast(),
                impersonation.0,
                desired_access,
                &mapping,
                privilege_words.as_mut_ptr().cast::<PRIVILEGE_SET>(),
                &mut privilege_length,
                &mut granted,
                &mut allowed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(allowed != 0 && granted & desired_access == desired_access)
    }

    fn reject_reparse(&self) -> io::Result<()> {
        let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            GetFileInformationByHandleEx(
                self.handle,
                FileAttributeTagInfo,
                std::ptr::addr_of_mut!(info).cast(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows sandbox capability targets may not be reparse points",
            ));
        }
        Ok(())
    }

    fn require_ntfs(&self) -> io::Result<()> {
        let mut filesystem = vec![0u16; 32];
        let ok = unsafe {
            GetVolumeInformationByHandleW(
                self.handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let length = filesystem
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(filesystem.len());
        let name = String::from_utf16_lossy(&filesystem[..length]);
        if !name.eq_ignore_ascii_case("NTFS") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Windows write protection requires local NTFS, found {name}"),
            ));
        }
        Ok(())
    }

    fn query_final_path(&self) -> io::Result<PathBuf> {
        let needed = unsafe { GetFinalPathNameByHandleW(self.handle, std::ptr::null_mut(), 0, 0) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u16; needed as usize + 1];
        let written = unsafe {
            GetFinalPathNameByHandleW(self.handle, buffer.as_mut_ptr(), buffer.len() as u32, 0)
        };
        if written == 0 || written as usize >= buffer.len() {
            return Err(io::Error::last_os_error());
        }
        let raw = std::ffi::OsString::from_wide(&buffer[..written as usize]);
        let text = raw.to_string_lossy();
        let ordinary = text.strip_prefix(r"\\?\").unwrap_or(&text);
        Ok(PathBuf::from(ordinary))
    }
}

/// `GetSecurityInfo` returns a self-relative descriptor, whereas AccessCheck
/// requires its absolute form. Keep every backing allocation alive for the
/// AccessCheck call.
#[cfg(test)]
struct AbsoluteSecurityDescriptor {
    descriptor: Vec<u8>,
    dacl: Vec<u8>,
    sacl: Vec<u8>,
    owner: Vec<u8>,
    group: Vec<u8>,
}

#[cfg(test)]
impl AbsoluteSecurityDescriptor {
    fn from_self_relative(input: *mut core::ffi::c_void) -> io::Result<Self> {
        let mut descriptor_size = 0;
        let mut dacl_size = 0;
        let mut sacl_size = 0;
        let mut owner_size = 0;
        let mut group_size = 0;
        // The sizing call is documented to fail with INSUFFICIENT_BUFFER.
        let ok = unsafe {
            MakeAbsoluteSD(
                input.cast(),
                std::ptr::null_mut(),
                &mut descriptor_size,
                std::ptr::null_mut(),
                &mut dacl_size,
                std::ptr::null_mut(),
                &mut sacl_size,
                std::ptr::null_mut(),
                &mut owner_size,
                std::ptr::null_mut(),
                &mut group_size,
            )
        };
        if ok != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return Err(io::Error::last_os_error());
        }
        let mut absolute = Self {
            descriptor: vec![0; descriptor_size as usize],
            dacl: vec![0; dacl_size as usize],
            sacl: vec![0; sacl_size as usize],
            owner: vec![0; owner_size as usize],
            group: vec![0; group_size as usize],
        };
        let ok = unsafe {
            MakeAbsoluteSD(
                input.cast(),
                absolute.descriptor.as_mut_ptr().cast(),
                &mut descriptor_size,
                ptr_or_null(&mut absolute.dacl).cast(),
                &mut dacl_size,
                ptr_or_null(&mut absolute.sacl).cast(),
                &mut sacl_size,
                ptr_or_null(&mut absolute.owner).cast(),
                &mut owner_size,
                ptr_or_null(&mut absolute.group).cast(),
                &mut group_size,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(absolute)
    }
}

#[cfg(test)]
fn ptr_or_null(bytes: &mut Vec<u8>) -> *mut u8 {
    if bytes.is_empty() {
        std::ptr::null_mut()
    } else {
        bytes.as_mut_ptr()
    }
}

impl Drop for FrozenPath {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.handle) };
        }
    }
}

#[cfg(test)]
struct HandleGuard(HANDLE);

#[cfg(test)]
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
struct LocalGuard(*mut core::ffi::c_void);

#[cfg(test)]
impl Drop for LocalGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0) };
        }
    }
}

fn reject_non_local_path(path: &Path) -> io::Result<()> {
    use std::path::Prefix;
    let Some(component) = path.components().next() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path"));
    };
    match component {
        std::path::Component::Prefix(prefix)
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
        {
            Ok(())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows sandbox capability requires an absolute local drive path",
        )),
    }
}

fn same_windows_path(expected: &Path, actual: &Path) -> bool {
    let normalize = |path: &Path| {
        path.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    normalize(expected) == normalize(actual)
}
