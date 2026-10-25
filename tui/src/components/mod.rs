//! UI components the workbench builds its surface from.
//!
//! What is left here is what the workbench actually draws with: the transcript
//! renderer (`chat_area`, with `diff` and `markdown` underneath it) and the
//! prompt box (`input`). The inline TUI's menus, panels, footer, pager and
//! dialogs went with it.

pub mod chat_area;
pub mod diff;
pub mod input;
pub mod markdown;
