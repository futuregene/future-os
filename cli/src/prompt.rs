//! A `[y/N]` question that one key answers.
//!
//! A confirmation that waits for Enter looks hung. `future remote devices remove
//! 2` asked `Unpair …? [y/N]`, the user pressed `y`, and the command sat at zero
//! CPU with nothing on disk: the prompt was waiting for a newline that nothing on
//! screen had asked for, so a typed `y` and a wedged process looked the same. On a
//! terminal the key itself is the answer — `y` or `Y` is yes, `n`, `N`, Enter and
//! Esc are no — while a redirected stdin still reads a line, which keeps
//! `printf 'y\n' | future …` scripts and the test harnesses working.
//!
//! The terminal is put in a mode that clears canonical input and echo and nothing
//! else: the signal keys stay, so Ctrl+C still aborts the command, and a guard
//! restores the previous mode on every path out, a panic included.

use future_agent::console;
use std::io::{IsTerminal, Write};

/// What one key decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Yes,
    No,
    /// Neither: an arrow key, a space, a half-typed word. A key that says
    /// nothing must not answer a question about destroying a pairing, so the
    /// prompt keeps waiting for one that means something.
    Ignore,
}

/// Ask `question` and read the answer.
///
/// `y`/`yes` in any case is consent and everything else is not; a bare Enter is
/// the documented no. The key is echoed here because echo is off while it is
/// read: without it the answer would never appear before the command's own
/// output, and the prompt would still look unanswered.
pub fn ask_yes_no(question: &str) -> Result<bool, String> {
    if !std::io::stdin().is_terminal() {
        return ask_line(question);
    }
    // The mode is switched before the question is printed, so a key typed the
    // instant the prompt appears is read as a key rather than left waiting for a
    // newline in the line buffer.
    #[cfg(any(unix, windows))]
    if let Some(raw) = terminal::Raw::enable() {
        write_prompt(question);
        let answer = read_keys(&raw);
        // A `yes` typed out of habit is answered by its first letter, and the
        // `es` behind it must not reach the shell as the next command. Discarded
        // while the terminal is still in this mode, before it is handed back.
        raw.discard_typeahead();
        // Restored before the answer is echoed, so the echo goes to a terminal
        // that is already back in its normal mode.
        drop(raw);
        console::eprintln!("{}", if answer { "y" } else { "n" });
        return Ok(answer);
    }
    // The terminal refused the mode change (an unusual tty, or a stdin that
    // stopped being one between the check and the call). A line still works, and
    // it is what every caller did before this existed.
    ask_line(question)
}

fn write_prompt(question: &str) {
    console::eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
}

fn ask_line(question: &str) -> Result<bool, String> {
    write_prompt(question);
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("Cannot read the answer: {error}"))?;
    Ok(is_yes(&answer))
}

/// What counts as consent from a line: `y` or `yes`, in any case. Everything
/// else — a bare Enter included — is a no.
fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// What one key means.
///
/// Esc is a no rather than something to ignore (it is how every other prompt in
/// this CLI is cancelled), which also makes an arrow key cancel: it arrives as
/// Esc followed by two more bytes. That is the harmless direction to be wrong in
/// — nothing is removed either way.
fn decide(key: char) -> Decision {
    match key {
        'y' | 'Y' => Decision::Yes,
        'n' | 'N' => Decision::No,
        // Enter takes the `[y/N]` default, and Esc and Ctrl+C abort: all three
        // leave the machine alone, which is what "no" means here.
        '\r' | '\n' | '\u{1b}' | '\u{3}' => Decision::No,
        _ => Decision::Ignore,
    }
}

/// Read keys until one of them decides; end of input is not consent.
#[cfg(any(unix, windows))]
fn read_keys(raw: &terminal::Raw) -> bool {
    loop {
        let Some(key) = raw.read() else {
            return false;
        };
        match decide(key) {
            Decision::Yes => return true,
            Decision::No => return false,
            Decision::Ignore => continue,
        }
    }
}

#[cfg(unix)]
mod terminal {
    /// `stdin` with canonical input and echo off, restored on drop.
    pub(super) struct Raw {
        original: libc::termios,
    }

    impl Raw {
        pub(super) fn enable() -> Option<Self> {
            // SAFETY: `tcgetattr` writes the struct it is handed, and its result
            // is checked before the value is read back.
            let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, original.as_mut_ptr()) } != 0 {
                return None;
            }
            // SAFETY: the call above returned 0, so the buffer is initialised.
            let original = unsafe { original.assume_init() };
            let mut raw = original;
            // ICANON is the newline wait this exists to remove, ECHO is what would
            // print the key back at us twice. ISIG stays: Ctrl+C must still abort.
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            // One byte is one key, with no timeout.
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            // SAFETY: `raw` is a fully initialised termios for this descriptor.
            if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
                return None;
            }
            Some(Self { original })
        }

        /// One key, or `None` at end of input.
        pub(super) fn read(&self) -> Option<char> {
            let mut byte = 0u8;
            // SAFETY: one byte is read into a buffer that outlives the call.
            let read = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
            if read != 1 {
                return None;
            }
            // Every key this prompt accepts is ASCII. A UTF-8 continuation byte
            // decodes to a `char` no arm of `decide` matches, so a non-ASCII key
            // is ignored rather than misread as one of them.
            Some(byte as char)
        }

        /// Drop what was typed but not read: the question is answered, and the
        /// rest of a habitual `yes` would otherwise be read by the shell as its
        /// next command.
        pub(super) fn discard_typeahead(&self) {
            // SAFETY: flushing the input queue of the descriptor already read.
            unsafe {
                libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
            }
        }
    }

    impl Drop for Raw {
        fn drop(&mut self) {
            // SAFETY: puts back the mode `enable` captured for this descriptor.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
            }
        }
    }
}

#[cfg(windows)]
mod terminal {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Console::{
        FlushConsoleInputBuffer, GetConsoleMode, GetStdHandle, ReadConsoleW, SetConsoleMode,
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, STD_INPUT_HANDLE,
    };

    /// Console input with line input and echo off, restored on drop.
    pub(super) struct Raw {
        handle: HANDLE,
        mode: u32,
    }

    impl Raw {
        pub(super) fn enable() -> Option<Self> {
            // SAFETY: each console call below reports its own failure, and every
            // result is checked before the next step depends on it.
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                if handle.is_null() {
                    return None;
                }
                let mut mode = 0u32;
                if GetConsoleMode(handle, &mut mode) == 0 {
                    return None;
                }
                // ENABLE_LINE_INPUT is the Enter wait, ENABLE_ECHO_INPUT is what
                // would print the key back. ENABLE_PROCESSED_INPUT stays, so
                // Ctrl+C is still the console's own and still aborts the command.
                if SetConsoleMode(handle, mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT)) == 0 {
                    return None;
                }
                Some(Self { handle, mode })
            }
        }

        /// One key, or `None` at end of input.
        pub(super) fn read(&self) -> Option<char> {
            let mut buffer = [0u16; 1];
            let mut read = 0u32;
            // SAFETY: a one-character read into a local buffer, on the console
            // handle `enable` opened, with the control block left to the system.
            let ok = unsafe {
                ReadConsoleW(
                    self.handle,
                    buffer.as_mut_ptr().cast(),
                    1,
                    &mut read,
                    std::ptr::null(),
                )
            };
            if ok == 0 || read != 1 {
                return None;
            }
            char::from_u32(u32::from(buffer[0]))
        }

        /// Drop what was typed but not read: the question is answered, and the
        /// rest of a habitual `yes` would otherwise be read by the shell as its
        /// next command.
        pub(super) fn discard_typeahead(&self) {
            // SAFETY: flushing the input queue of the handle already read.
            unsafe {
                FlushConsoleInputBuffer(self.handle);
            }
        }
    }

    impl Drop for Raw {
        fn drop(&mut self) {
            // SAFETY: puts back the mode `enable` captured for this handle.
            unsafe {
                SetConsoleMode(self.handle, self.mode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A keypress answers, and only the two letters do: the point of the mode
    /// change is that Enter is not needed, and it is not trusted either.
    #[test]
    fn only_the_letters_decide() {
        for key in ['y', 'Y'] {
            assert_eq!(decide(key), Decision::Yes, "{key:?}");
        }
        for key in ['n', 'N'] {
            assert_eq!(decide(key), Decision::No, "{key:?}");
        }
        // The prompt's documented default plus the two aborts.
        for key in ['\r', '\n', '\u{1b}', '\u{3}'] {
            assert_eq!(decide(key), Decision::No, "{key:?}");
        }
        // Anything else keeps the prompt waiting, so a stray key cannot remove a
        // device: only a key that means something answers.
        for key in [' ', 'x', '1', '\u{7f}', 'é'] {
            assert_eq!(decide(key), Decision::Ignore, "{key:?}");
        }
    }

    /// The line-shaped answer a pipe (or a caller without a terminal) gives is
    /// still read by the same rule.
    #[test]
    fn a_line_is_still_a_yes_or_no() {
        for yes in ["y", "Y", "yes", "YES", " yes \n"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "\n", "n", "no", "nope", "yess", "1"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }
}
