//! Child-process helpers.
//!
//! The GUI is built with `windows_subsystem = "windows"` (no console). Spawning a
//! console program (`git.exe`, `cmd.exe`) via `std::process::Command` then makes
//! Windows allocate a fresh console for the child, which flashes on screen as a
//! black command-prompt window. Passing `CREATE_NO_WINDOW` suppresses it.
//!
//! Apply `.no_window()` to every `Command` we build directly. (Tauri's sidecar
//! path already sets this flag, so the bundled agent is unaffected.)

/// Suppress the transient console window Windows creates for a spawned console
/// child. No-op on non-Windows targets.
pub trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    #[cfg(windows)]
    fn no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW` — the child runs without allocating a console.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.creation_flags(CREATE_NO_WINDOW)
    }

    #[cfg(not(windows))]
    fn no_window(&mut self) -> &mut Self {
        self
    }
}
