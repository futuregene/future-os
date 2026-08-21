//! Suspended restricted-token process creation for W3.
//!
//! The process is created suspended with an explicit inherited-handle list,
//! assigned to a no-breakaway Job Object, and only then resumed. Any failure
//! before `ResumeThread` terminates the still-suspended process fail-closed.

#![allow(dead_code)] // Product wiring remains disabled until W4/W6.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::Path;
use std::ptr;
use std::sync::Arc;

use uuid::Uuid;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, SetHandleInformation, ERROR_SUCCESS, HANDLE, HANDLE_FLAG_INHERIT,
    WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    SetEntriesInAclW, SetSecurityInfo, EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE,
    SE_WINDOW_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CREATEMENU, DESKTOP_CREATEWINDOW, DESKTOP_DELETE,
    DESKTOP_ENUMERATE, DESKTOP_HOOKCONTROL, DESKTOP_JOURNALPLAYBACK, DESKTOP_JOURNALRECORD,
    DESKTOP_READOBJECTS, DESKTOP_READ_CONTROL, DESKTOP_SWITCHDESKTOP, DESKTOP_WRITEOBJECTS,
    DESKTOP_WRITE_DAC, DESKTOP_WRITE_OWNER, HDESK,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use super::token::RestrictedToken;
use super::Job;

pub(crate) struct RestrictedChild {
    process: Arc<ProcessHandle>,
    job: Job,
    // Must outlive the child: the restricted process initializes against this
    // private desktop after ResumeThread.
    _desktop: PrivateDesktop,
    stdout: Option<File>,
    stderr: Option<File>,
    pid: u32,
}

impl RestrictedChild {
    pub(crate) fn spawn(
        token: &RestrictedToken,
        program: &OsStr,
        args: &[OsString],
        cwd: &Path,
        env_overrides: &[(OsString, OsString)],
    ) -> io::Result<Self> {
        if !cwd.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "restricted process cwd must be absolute",
            ));
        }

        let (stdout_read, stdout_write) = inheritable_pipe()
            .map_err(|error| io::Error::other(format!("create stdout pipe: {error}")))?;
        let (stderr_read, stderr_write) = inheritable_pipe()
            .map_err(|error| io::Error::other(format!("create stderr pipe: {error}")))?;
        let stdin = open_inheritable_null()
            .map_err(|error| io::Error::other(format!("open inheritable NUL: {error}")))?;
        let inherited = [stdin.raw(), stdout_write.raw(), stderr_write.raw()];
        let attributes = AttributeList::with_handle_list(&inherited)
            .map_err(|error| io::Error::other(format!("build attribute list: {error}")))?;
        let desktop = PrivateDesktop::create(token)
            .map_err(|error| io::Error::other(format!("create private desktop: {error}")))?;

        let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdin.raw();
        startup.StartupInfo.hStdOutput = stdout_write.raw();
        startup.StartupInfo.hStdError = stderr_write.raw();
        startup.lpAttributeList = attributes.pointer();

        let mut command_line = build_command_line(program, args);
        let cwd_wide = wide_nul(cwd.as_os_str());
        startup.StartupInfo.lpDesktop = desktop.startup_name.as_ptr().cast_mut();
        let environment = build_environment_block(env_overrides)?;
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: all UTF-16 buffers are NUL-terminated and live through this
        // synchronous call. The mutable command-line buffer is required by the
        // Win32 API. Only the three handles in the attribute list are inherited.
        let ok = unsafe {
            CreateProcessAsUserW(
                token.as_handle(),
                ptr::null(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_SUSPENDED
                    | CREATE_UNICODE_ENVIRONMENT
                    | CREATE_NO_WINDOW
                    | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr().cast(),
                cwd_wide.as_ptr(),
                ptr::addr_of!(startup.StartupInfo),
                &mut info,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            return Err(io::Error::other(format!(
                "CreateProcessAsUserW failed for {program:?} (os error {error})"
            )));
        }

        let process = OwnedHandle(info.hProcess);
        let thread = OwnedHandle(info.hThread);
        let job = match Job::create_sandbox().and_then(|job| {
            job.assign_handle(process.raw())?;
            Ok(job)
        }) {
            Ok(job) => job,
            Err(error) => {
                terminate_suspended(process.raw());
                return Err(error);
            }
        };
        // SAFETY: the primary thread is still suspended and owned here.
        if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
            let error = io::Error::last_os_error();
            job.terminate();
            return Err(error);
        }

        // Parent copies of child-only handles must close immediately so EOF is
        // observable once the restricted process tree exits.
        drop(stdin);
        drop(stdout_write);
        drop(stderr_write);
        drop(thread);
        Ok(Self {
            process: Arc::new(ProcessHandle(process)),
            job,
            _desktop: desktop,
            stdout: Some(stdout_read.into_file()),
            stderr: Some(stderr_read.into_file()),
            pid: info.dwProcessId,
        })
    }

    pub(crate) fn id(&self) -> u32 {
        self.pid
    }

    pub(crate) fn take_stdout(&mut self) -> Option<File> {
        self.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<File> {
        self.stderr.take()
    }

    pub(crate) fn terminate(&self) {
        self.job.terminate();
    }

    pub(crate) async fn wait(&self) -> io::Result<u32> {
        let process = Arc::clone(&self.process);
        let result = tokio::task::spawn_blocking(move || process.wait())
            .await
            .map_err(|error| io::Error::other(format!("process wait task failed: {error}")))?;
        // The shell has exited; terminate any descendants still holding stdio
        // handles so output readers observe EOF. Sandbox jobs never support the
        // legacy detached-browser behavior.
        self.job.terminate();
        result
    }
}

/// A private desktop on the interactive `Winsta0` window station grants the
/// restricted token's capability trustees enough USER-object rights for
/// DLL/console initialization. It avoids adding the real User SID to the
/// token's restricting SID set, which would otherwise broaden filesystem writes
/// through ordinary user ACLs. The logon SID and Everyone that the token carries
/// for PowerShell/CLR compatibility are deliberately not granted access to the
/// desktop.
///
/// `CreateWindowStationW` returned ERROR_ACCESS_DENIED on the current
/// unelevated Windows test host, so a dedicated station is not a compatible
/// baseline. Instead this creates a uniquely named desktop on the
/// caller's existing `Winsta0` station — the same approach as Codex's legacy
/// backend. `Winsta0`'s DACL already grants the ordinary token's user/groups
/// the read rights `CreateProcessAsUserW` needs to attach the child; the
/// desktop itself carries the normal-user + restricting-SID ACL so the
/// WRITE_RESTRICTED access check on USER objects also passes.
struct PrivateDesktop {
    desktop: HDESK,
    startup_name: Vec<u16>,
}

// HDESK is an owned Win32 handle. This owner never calls SetThreadDesktop; after
// creation the handle is only used to set the DACL synchronously and then closed
// when the child is torn down. Windows permits that handle ownership/close to
// move across Tokio worker threads.
unsafe impl Send for PrivateDesktop {}
unsafe impl Sync for PrivateDesktop {}

impl PrivateDesktop {
    fn create(token: &RestrictedToken) -> io::Result<Self> {
        let identity = Uuid::new_v4().simple().to_string();
        let desktop_name = format!("FutureOSSandbox-{identity}");
        let desktop_name_wide = wide_nul(OsStr::new(&desktop_name));

        // Match the rights Codex gives its private desktop. This is not a
        // filesystem capability: the desktop is freshly created for this child
        // and its handle closes with the child. A narrower guessed subset made
        // PowerShell fail during DLL initialization on clean Windows 11.
        let access = DESKTOP_READOBJECTS
            | DESKTOP_CREATEWINDOW
            | DESKTOP_CREATEMENU
            | DESKTOP_HOOKCONTROL
            | DESKTOP_JOURNALRECORD
            | DESKTOP_JOURNALPLAYBACK
            | DESKTOP_ENUMERATE
            | DESKTOP_WRITEOBJECTS
            | DESKTOP_SWITCHDESKTOP
            | DESKTOP_DELETE
            | DESKTOP_READ_CONTROL
            | DESKTOP_WRITE_DAC
            | DESKTOP_WRITE_OWNER;
        // CreateDesktopW with a null station targets the caller's current
        // process window station, i.e. the interactive `Winsta0`.
        let desktop = unsafe {
            CreateDesktopW(
                desktop_name_wide.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                0,
                access,
                ptr::null(),
            )
        };
        if desktop.is_null() {
            return Err(io::Error::other(format!(
                "CreateDesktopW failed: {}",
                io::Error::last_os_error()
            )));
        }
        let desktop = OwnedDesktop(desktop);
        grant_user_object_access(desktop.0, token, access)
            .map_err(|error| io::Error::other(format!("grant desktop access: {error}")))?;

        Ok(Self {
            desktop: desktop.into_raw(),
            startup_name: wide_nul(OsStr::new(&format!("Winsta0\\{desktop_name}"))),
        })
    }
}

impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        if !self.desktop.is_null() {
            unsafe { CloseDesktop(self.desktop) };
        }
    }
}

fn grant_user_object_access(
    handle: HANDLE,
    token: &RestrictedToken,
    access: u32,
) -> io::Result<()> {
    // WRITE_RESTRICTED performs two checks. The ordinary token check needs the
    // current user's SID; the restricted check needs a capability SID. These
    // ACEs exist only on the short-lived USER object. Never add the real user
    // SID to SidsToRestrict: that could authorize writes on ordinary user ACLs.
    let entries = std::iter::once(token.normal_user_sid())
        .chain(token.capability_sids())
        .map(|sid| EXPLICIT_ACCESS_W {
            grfAccessPermissions: access,
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
    let mut updated = ptr::null_mut();
    let status = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            ptr::null(),
            &mut updated,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let updated = LocalAcl(updated);
    let status = unsafe {
        SetSecurityInfo(
            handle,
            SE_WINDOW_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            updated.0,
            ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

struct OwnedDesktop(HDESK);

impl OwnedDesktop {
    fn into_raw(mut self) -> HDESK {
        std::mem::replace(&mut self.0, ptr::null_mut())
    }
}

impl Drop for OwnedDesktop {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseDesktop(self.0) };
        }
    }
}

struct LocalAcl(*mut windows_sys::Win32::Security::ACL);

impl Drop for LocalAcl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0.cast()) };
        }
    }
}

struct ProcessHandle(OwnedHandle);

impl ProcessHandle {
    fn wait(&self) -> io::Result<u32> {
        let result = unsafe { WaitForSingleObject(self.0.raw(), INFINITE) };
        if result == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }
        if result != WAIT_OBJECT_0 {
            return Err(io::Error::other(format!(
                "unexpected process wait result: {result}"
            )));
        }
        let mut exit_code = 0;
        if unsafe { GetExitCodeProcess(self.0.raw(), &mut exit_code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(exit_code)
    }
}

struct OwnedHandle(HANDLE);

// Kernel object handles may be waited/closed from any thread; ownership is
// unique except for the explicit Arc around the process handle.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }

    fn into_file(mut self) -> File {
        let handle = std::mem::replace(&mut self.0, ptr::null_mut());
        // SAFETY: ownership of this valid pipe handle transfers to File.
        unsafe { File::from_raw_handle(handle as RawHandle) }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn inheritable_pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let read = OwnedHandle(read);
    let write = OwnedHandle(write);
    // Only the child write end is inherited.
    if unsafe { SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((read, write))
}

fn open_inheritable_null() -> io::Result<OwnedHandle> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let name = wide_nul(OsStr::new("NUL"));
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_GENERIC_READ | FILE_GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedHandle(handle))
}

struct AttributeList {
    storage: Vec<usize>,
    pointer: *mut core::ffi::c_void,
}

impl AttributeList {
    fn with_handle_list(handles: &[HANDLE]) -> io::Result<Self> {
        let mut bytes = 0usize;
        // The first call intentionally fails and reports required storage.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let words = bytes.div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0usize; words];
        let pointer = storage.as_mut_ptr().cast();
        if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut bytes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let list = Self { storage, pointer };
        let ok = unsafe {
            UpdateProcThreadAttribute(
                list.pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(list)
    }

    fn pointer(&self) -> *mut core::ffi::c_void {
        self.pointer
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.pointer) };
    }
}

fn terminate_suspended(process: HANDLE) {
    unsafe {
        TerminateProcess(process, 1);
        WaitForSingleObject(process, INFINITE);
    }
}

fn build_command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    let mut command = Vec::new();
    append_quoted_arg(&mut command, program);
    for arg in args {
        command.push(b' ' as u16);
        append_quoted_arg(&mut command, arg);
    }
    command.push(0);
    command
}

/// Quote one argv element using the CommandLineToArgvW-compatible backslash
/// rules used by the Microsoft C runtime.
fn append_quoted_arg(output: &mut Vec<u16>, arg: &OsStr) {
    let units: Vec<u16> = arg.encode_wide().collect();
    let quoted = units.is_empty()
        || units
            .iter()
            .any(|unit| *unit == 0x20 || *unit == 0x09 || *unit == b'"' as u16);
    if !quoted {
        output.extend(units);
        return;
    }
    output.push(b'"' as u16);
    let mut slashes = 0usize;
    for unit in units {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else if unit == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            output.push(unit);
            slashes = 0;
        } else {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            output.push(unit);
            slashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
}

fn build_environment_block(overrides: &[(OsString, OsString)]) -> io::Result<Vec<u16>> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (key, value) in overrides {
        if key.is_empty() || key.to_string_lossy().contains(['=', '\0']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Windows environment variable name",
            ));
        }
        if value.to_string_lossy().contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows environment variable value contains NUL",
            ));
        }
        let key_folded = key.to_string_lossy().to_lowercase();
        if let Some(existing) = vars
            .iter_mut()
            .find(|(candidate, _)| candidate.to_string_lossy().to_lowercase() == key_folded)
        {
            *existing = (key.clone(), value.clone());
        } else {
            vars.push((key.clone(), value.clone()));
        }
    }
    vars.sort_by(|(left, _), (right, _)| {
        left.to_string_lossy()
            .to_lowercase()
            .cmp(&right.to_string_lossy().to_lowercase())
    });
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(key.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn wide_nul(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn restricted_child_stays_send_for_shell_futures() {
        assert_send_sync::<RestrictedChild>();
    }

    fn decode(line: Vec<u16>) -> String {
        String::from_utf16_lossy(&line[..line.len() - 1])
    }

    #[test]
    fn command_line_quotes_spaces_quotes_and_trailing_slashes() {
        let line = build_command_line(
            OsStr::new(r"C:\Program Files\pwsh.exe"),
            &[
                OsString::from("plain"),
                OsString::from("two words"),
                OsString::from("quoted\"value"),
                OsString::from(r"C:\tail\"),
            ],
        );
        assert_eq!(
            decode(line),
            r#""C:\Program Files\pwsh.exe" plain "two words" "quoted\"value" C:\tail\"#
        );
    }

    #[test]
    fn environment_block_is_double_nul_terminated() {
        let block =
            build_environment_block(&[(OsString::from("FutureOS_W3"), OsString::from("值"))])
                .unwrap();
        assert_eq!(&block[block.len() - 2..], &[0, 0]);
        let text = String::from_utf16_lossy(&block);
        assert!(text.contains("FutureOS_W3=值\0"));
    }
}
