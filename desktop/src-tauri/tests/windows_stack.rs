//! The Windows desktop reserves the stack its command dispatch needs.
//!
//! Windows takes a process's main-thread stack size from its PE header, and the
//! linker default (1 MB) is an eighth of what Linux and macOS reserve. Tauri
//! builds an async command's future on the thread that received the IPC
//! message and needs several copies of it at once — the app's largest future
//! (`agent_prompt`, 143,736 bytes) asked for a 575,856-byte frame — which
//! overflowed that 1 MB and killed the process with `STATUS_STACK_OVERFLOW`
//! (0xc00000fd) as soon as a prompt was sent. `build.rs` passes `/STACK` for
//! this; assert on the built binary's own header so losing that link argument
//! fails here rather than as a silent crash in a user's app.
#![cfg(all(windows, feature = "gui"))]

use std::path::Path;

#[test]
fn the_desktop_binary_uses_the_windows_gui_subsystem() {
    let image = std::fs::read(Path::new(env!("CARGO_BIN_EXE_futureos")))
        .expect("read the built desktop binary");
    let pe = u32::from_le_bytes(image[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&image[pe..pe + 4], b"PE\0\0", "not a PE image");
    let optional = pe + 24;
    let subsystem = u16::from_le_bytes(image[optional + 0x44..optional + 0x46].try_into().unwrap());
    assert_eq!(
        subsystem, 2,
        "Desktop must use IMAGE_SUBSYSTEM_WINDOWS_GUI to avoid a console flashing at launch"
    );
}

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
fn the_desktop_binary_reserves_the_platform_stack() {
    let image = std::fs::read(Path::new(env!("CARGO_BIN_EXE_futureos")))
        .expect("read the built desktop binary");
    let reserve = stack_reserve(&image);
    assert!(
        reserve >= 8 * 1024 * 1024,
        "the desktop binary reserves {reserve} bytes of stack; a Tauri IPC dispatch of a large \
         command future needs far more than the 1 MB linker default (see build.rs)"
    );
}
