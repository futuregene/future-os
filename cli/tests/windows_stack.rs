//! The Windows CLI reserves the stack its commands run on.
//!
//! Windows takes a process's main-thread stack size from its PE header, and the
//! linker default (1 MB) is an eighth of what Linux and macOS give the same
//! code. Growth is on demand, so the difference only shows up when a command
//! needs a frame past the reserve — and then it is fatal:
//! `STATUS_STACK_OVERFLOW` (0xc00000fd), no message, no exit path.
//!
//! That is not hypothetical here: `future session list` died exactly that way on
//! one build of this CLI while `models list`, `task list` and `auth status` all
//! worked, which is what a reserve that is barely big enough looks like. The
//! desktop already reserves 8 MB for its command dispatch
//! (`desktop/src-tauri/build.rs` and `desktop/src-tauri/tests/windows_stack.rs`);
//! `build.rs` passes the same argument for this binary. This asserts it on the
//! built image, so losing the link argument fails here rather than in a user's
//! shell.
#![cfg(windows)]

use std::path::Path;

/// `SizeOfStackReserve` from the PE32+ optional header of an image.
fn stack_reserve(image: &[u8]) -> u64 {
    let pe = u32::from_le_bytes(image[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&image[pe..pe + 4], b"PE\0\0", "not a PE image");
    let optional = pe + 24; // past the COFF header
    let magic = u16::from_le_bytes(image[optional..optional + 2].try_into().unwrap());
    assert_eq!(magic, 0x20b, "expected a PE32+ image");
    // SizeOfStackReserve sits at the same offset in both optional-header
    // flavours; PE32+ widens it to 64 bits.
    u64::from_le_bytes(image[optional + 0x48..optional + 0x50].try_into().unwrap())
}

#[test]
fn the_cli_binary_reserves_the_platform_stack() {
    let image = std::fs::read(Path::new(env!("CARGO_BIN_EXE_future"))).expect("read the built CLI");
    let reserve = stack_reserve(&image);
    assert!(
        reserve >= 8 * 1024 * 1024,
        "the CLI reserves {reserve} bytes of stack, the Windows linker default; a command whose \
         future outgrows it dies with STATUS_STACK_OVERFLOW and no message (see build.rs)"
    );
}
