//! What the OS says this machine is called.
//!
//! The label this desktop shows on another machine's device list is built from
//! the machine's own name. That name used to be read from `COMPUTERNAME` /
//! `HOSTNAME`, which is only a *forwarded* copy: a launcher that decodes the
//! value with a local code page turns a Chinese computer name into
//! `␦␦␦␦␦␦ļ␦` (U+FFFD, one per lost byte), and the label then names the
//! device worse than leaving it unnamed. Asking the OS for the name skips every
//! forwarding hop: the value never leaves UTF-16 / CFString, so it cannot be
//! re-encoded on the way.
//!
//! Each platform answers with the name a person would recognise — the macOS
//! *Computer Name* the user set in System Settings, not the DNS host name.

/// The machine's user-facing name, or `None` when this platform cannot say.
///
/// `None` is not a failure: the caller falls back to the environment copy and
/// ultimately to an unnamed device, which the UI has wording for.
pub(super) fn computer_name() -> Option<String> {
    platform::computer_name()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;

    #[link(name = "SystemConfiguration", kind = "framework")]
    extern "C" {
        /// The user-assigned Computer Name, or null when the store has none.
        /// The string is owned by the caller.
        fn SCDynamicStoreCopyComputerName(
            store: *const c_void,
            name_encoding: *mut u32,
        ) -> *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetLength(value: *const c_void) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            value: *const c_void,
            buffer: *mut i8,
            buffer_size: isize,
            encoding: u32,
        ) -> bool;
        fn CFRelease(value: *const c_void);
    }

    /// `kCFStringEncodingUTF8`: the conversion CoreFoundation does itself, so
    /// the name reaches Rust as UTF-8 rather than through a code page.
    const UTF8: u32 = 0x0800_0100;

    pub(super) fn computer_name() -> Option<String> {
        // SAFETY: plain C calls with no aliasing. The string arrives +1 and is
        // released here; the buffer is sized by CoreFoundation for this string
        // in this encoding, so the copy cannot overrun it.
        unsafe {
            let name = SCDynamicStoreCopyComputerName(std::ptr::null(), std::ptr::null_mut());
            if name.is_null() {
                return None;
            }
            let needed = CFStringGetMaximumSizeForEncoding(CFStringGetLength(name), UTF8) + 1;
            let mut buffer = vec![0i8; needed.max(1) as usize];
            let copied = CFStringGetCString(name, buffer.as_mut_ptr(), needed, UTF8);
            CFRelease(name);
            if !copied {
                return None;
            }
            let bytes: Vec<u8> = buffer
                .into_iter()
                .take_while(|byte| *byte != 0)
                .map(|byte| byte as u8)
                .collect();
            String::from_utf8(bytes).ok()
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;

    pub(super) fn computer_name() -> Option<String> {
        let mut buffer = [0u16; 256];
        let mut size = buffer.len() as u32;
        // SAFETY: the API writes at most `size` UTF-16 units into `buffer` and
        // reports how many it wrote in `size`.
        let ok = unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) };
        if ok == 0 {
            return None;
        }
        // A computer name is decoded by Windows, not by us: `from_utf16` fails
        // rather than replacing, so a name this process cannot represent is
        // reported as unavailable instead of as a label full of U+FFFD.
        String::from_utf16(&buffer[..size as usize]).ok()
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    pub(super) fn computer_name() -> Option<String> {
        // The kernel's own value: no launcher, no code page, no environment.
        // Linux publishes it as a file; on other unices there is nothing to
        // read here, and the caller falls back to the environment copy.
        if !cfg!(target_os = "linux") {
            return None;
        }
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
}
