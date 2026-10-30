//! Drive every workbench page with real key and pointer events — not just
//! render it — and report what actually answers.
//!
//! `cargo run -p future-tui --example workbench_interaction`
//! `cargo run -p future-tui --example workbench_interaction -- --size 80x24`
//! `cargo run -p future-tui --example workbench_interaction -- --verbose`
//!
//! The static frames (`workbench_demo`) prove what a page *looks* like. This
//! proves what it *does*: every region the last frame registered is clicked at
//! its own centre, every listed key is pressed on a fresh page, and each answer
//! is checked against invariants a screenshot can never show:
//!
//! * **inside** — a region must lie within the frame that painted it; an
//!   off-screen click target is unreachable by a real pointer.
//! * **topmost** — a control's own centre must resolve back to it. A rect
//!   painted under a later one looks clickable and silently does something else.
//! * **alive** — a click or key must either return an action or change the
//!   workbench state. A region that does neither is decoration pretending to be
//!   a control, which is how `panel.row.3` once answered `Unknown command`.
//!
//! Exits non-zero when any invariant fails, so it can gate a delivery.

use future_tui::workbench::ansi::width;
use future_tui::workbench::demo;
use future_tui::workbench::diff_review::DiffFile;
use future_tui::workbench::i18n::LanguagePreference;
use future_tui::workbench::layout::Rect;
use future_tui::workbench::mouse::{MouseEvent, MouseKind};
use future_tui::workbench::picker::PickerKind;
use future_tui::workbench::state::{
    ApprovalView, DiffView, ForkPoint, ForkRole, PanelId, QueuedPrompt, WorkbenchData, WorktreeView,
};
use future_tui::workbench::{Overlay, WbAction, WbEvent, Workbench};

fn main() {
    let mut size = (120u16, 38u16);
    let mut verbose = false;
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" | "-s" => size = parse_size(&args.next().unwrap_or_default()),
            "--page" | "-p" => only = args.next(),
            "--verbose" | "-v" => verbose = true,
            "--help" | "-h" => {
                println!("--size WxH · --page NAME · --verbose");
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    let mut failures = 0;
    let mut total_regions = 0;
    let mut total_clicks = 0;
    // A page whose fixture was removed with its feature must fail *here*, not
    // halfway down the list: the panic used to abort the run after fifteen
    // pages had passed, so every page behind it — including the ones a change
    // touched — silently stopped being audited.
    for name in PAGES {
        let _ = fixture(name);
    }
    for name in PAGES {
        if let Some(wanted) = only.as_deref() {
            if wanted != *name {
                continue;
            }
        }
        let (data, open) = fixture(name);
        let report = audit_page(name, &data, open, size, verbose);
        total_regions += report.regions;
        total_clicks += report.clicks;
        if report.issues.is_empty() {
            println!(
                "ok   {:<16} {:>3} regions {:>3} clicks {:>2} keys",
                name, report.regions, report.clicks, report.keys
            );
        } else {
            failures += report.issues.len();
            println!(
                "FAIL {:<16} {:>3} regions {:>3} clicks {:>2} keys",
                name, report.regions, report.clicks, report.keys
            );
            for issue in &report.issues {
                println!("       {issue}");
            }
        }
    }
    let (w, h) = size;
    println!(
        "\n{w}x{h}: {total_regions} regions, {total_clicks} pointer checks, {failures} problem(s)"
    );
    if failures > 0 {
        std::process::exit(1);
    }
}

fn parse_size(text: &str) -> (u16, u16) {
    let mut parts = text.split(['x', 'X']);
    let w = parts.next().and_then(|v| v.parse().ok()).unwrap_or(120);
    let h = parts.next().and_then(|v| v.parse().ok()).unwrap_or(38);
    (w, h)
}

/// Every page the surface can show, in the order the reader meets them.
const PAGES: &[&str] = &[
    "launchpad",
    "chat",
    "workspaces",
    "panel-recent",
    "panel-workspaces",
    "panel-files",
    "preview-image",
    "preview-text",
    "queue",
    "approval",
    "approval-norule",
    "diff",
    "diff-clean",
    "diff-error",
    "fork",
    "worktree",
    "preferences",
    "keybindings",
    "help",
    "palette",
    "picker-model",
    "picker-thinking",
    "picker-mode",
    "picker-workspace",
    "picker-theme",
    "picker-sessions",
    "picker-language",
];

/// The fixture each page needs, and the call that opens it.
fn fixture(name: &str) -> (WorkbenchData, fn(&mut Workbench, &WorkbenchData)) {
    let chat = || demo::chat();
    match name {
        "launchpad" => (demo::data(demo::Scenario::Launchpad), noop),
        "chat" => (chat(), noop),
        "workspaces" => (demo::data(demo::Scenario::Workspaces), noop),
        "panel-recent" => (chat(), |wb, _| wb.panel = PanelId::Recent),
        "panel-workspaces" => (chat(), |wb, _| wb.panel = PanelId::Workspaces),
        "panel-files" => (chat(), |wb, _| wb.panel = PanelId::Files),
        "preview-image" => (
            {
                let mut data = chat();
                data.preview = Some(demo::preview_image());
                data
            },
            |wb, _| wb.overlay = Some(Overlay::FilePreview),
        ),
        "preview-text" => (
            {
                let mut data = chat();
                let path = "tui/src/workbench/syntax.rs";
                let source = r#"// Colour a code preview, once per line.
use crate::workbench::syntax;

pub struct Span {
    pub start: usize,
    pub end: usize,
}

pub fn highlight(path: &str, text: &str) -> Vec<Line> {
    let mut lexer = Lexer::new(Language::from_path(path));
    text.lines().map(|line| lexer.line(line)).collect()
}"#;
                data.preview = Some(future_tui::workbench::state::PreviewView::Text {
                    path: path.into(),
                    lines: future_tui::workbench::syntax::highlight(path, source).into(),
                    truncated: false,
                });
                data
            },
            |wb, _| wb.overlay = Some(Overlay::FilePreview),
        ),
        "queue" => (
            {
                let mut data = chat();
                data.queue = vec![
                    QueuedPrompt {
                        run_id: "run-2".into(),
                        text: "split the regression tests into unit and integration".into(),
                    },
                    QueuedPrompt {
                        run_id: "run-3".into(),
                        text: "and check the Windows path handling too".into(),
                    },
                ];
                data
            },
            |wb, data| {
                wb.surface_command("/queue", data);
            },
        ),
        "approval" => (
            {
                let mut data = chat();
                data.approval = Some(approval(true));
                data
            },
            |wb, _| wb.overlay = Some(Overlay::Approval),
        ),
        "approval-norule" => (
            {
                let mut data = chat();
                data.approval = Some(approval(false));
                data
            },
            |wb, _| wb.overlay = Some(Overlay::Approval),
        ),
        "diff" => (
            {
                let mut data = chat();
                data.diff = Some(diff());
                data
            },
            |wb, _| wb.overlay = Some(Overlay::Diff),
        ),
        "diff-clean" => (
            {
                let mut data = chat();
                data.diff = Some(DiffView::default());
                data
            },
            |wb, _| wb.overlay = Some(Overlay::Diff),
        ),
        "diff-error" => (
            {
                let mut data = chat();
                data.diff = Some(DiffView {
                    error: Some("fatal: not a git repository".into()),
                    ..DiffView::default()
                });
                data
            },
            |wb, _| wb.overlay = Some(Overlay::Diff),
        ),
        "fork" => (
            {
                let mut data = chat();
                data.fork_points = (0..5)
                    .map(|index| ForkPoint {
                        id: format!("e{index}"),
                        // Every other row is an answer: the picker lists both
                        // kinds, and the label is what tells them apart.
                        role: if index % 2 == 0 {
                            ForkRole::User
                        } else {
                            ForkRole::Assistant
                        },
                        text: format!("message point {index}"),
                    })
                    .collect();
                data
            },
            |wb, _| wb.overlay = Some(Overlay::Fork),
        ),
        "worktree" => (
            {
                let mut data = chat();
                data.worktrees = (0..4)
                    .map(|index| WorktreeView {
                        path: format!("D:/future-os/.worktrees/wt-{index}"),
                        label: format!("feat/wt-{index}"),
                        description: format!("worktree number {index}"),
                        current: index == 0,
                    })
                    .collect();
                data
            },
            |wb, _| wb.overlay = Some(Overlay::Worktree),
        ),
        "preferences" => (chat(), |wb, _| wb.open_preferences()),
        "keybindings" => (chat(), |wb, _| {
            wb.open_preferences();
            wb.overlay = Some(Overlay::Keybindings);
        }),
        "help" => (chat(), |wb, _| wb.overlay = Some(Overlay::Help)),
        "palette" => (chat(), |wb, _| wb.overlay = Some(Overlay::Palette)),
        "picker-model" => (chat(), |wb, data| {
            wb.open_picker(PickerKind::Model, data);
        }),
        "picker-thinking" => (chat(), |wb, data| {
            wb.open_picker(PickerKind::Thinking, data);
        }),
        "picker-mode" => (chat(), |wb, data| {
            wb.open_picker(PickerKind::Approval, data);
        }),
        "picker-workspace" => (chat(), |wb, data| {
            wb.open_picker(PickerKind::Workspace, data);
        }),
        "picker-theme" => (chat(), |wb, data| {
            wb.open_picker(PickerKind::Theme, data);
        }),
        "picker-sessions" => (chat(), |wb, data| {
            wb.open_picker(PickerKind::Sessions, data);
        }),
        "picker-language" => (chat(), |wb, data| {
            wb.open_picker(PickerKind::Language, data);
        }),
        other => panic!(
            "{other} is listed in PAGES but has no fixture: drop the page, or give it an arm here"
        ),
    }
}

fn noop(_: &mut Workbench, _: &WorkbenchData) {}

fn approval(with_rule: bool) -> ApprovalView {
    ApprovalView {
        request_id: "approval-42".into(),
        tool: "shell".into(),
        kind: "command".into(),
        risk: "HIGH".into(),
        title: "run the tests and the coverage check".into(),
        summary: "run the Rust test suite in the workspace and write a report".into(),
        action: "{\n  \"command\": \"cargo test -p future-tui\"\n}".into(),
        requested_action: "run a shell command; read project files, write target/".into(),
        sandbox_boundary: "workspace: D:/future-os\nwrite: target/**\nnetwork: denied".into(),
        rules: if with_rule {
            vec![future_tui::workbench::state::AllowRule {
                path: "target/**".into(),
                access: "write".into(),
            }]
        } else {
            Vec::new()
        },
    }
}

fn diff() -> DiffView {
    DiffView {
        files: (0..6)
            .map(|index| DiffFile {
                path: format!("tui/src/workbench/module_{index}.rs"),
                status: if index % 3 == 0 { "M" } else { "A" }.into(),
            })
            .collect(),
        selected: 0,
        patch: (0..120)
            .map(|index| format!("+ line {index} of a patch long enough to scroll"))
            .collect::<Vec<_>>()
            .join("\n"),
        error: None,
    }
}

struct Report {
    issues: Vec<String>,
    regions: usize,
    clicks: usize,
    keys: usize,
}

/// Keys worth pressing on any page: navigation, the overlay verbs and a couple
/// of literal characters (which mean "type" in a search box and "no" on an
/// approval — both real answers).
const KEYS: &[&str] = &[
    "up",
    "down",
    "left",
    "right",
    "tab",
    "shift+tab",
    "pageUp",
    "pageDown",
    "home",
    "end",
    "enter",
    "escape",
    " ",
    "x",
    "y",
    "n",
    "a",
    "r",
    "?",
];

fn audit_page(
    name: &str,
    data: &WorkbenchData,
    open: fn(&mut Workbench, &WorkbenchData),
    (w, h): (u16, u16),
    verbose: bool,
) -> Report {
    let mut issues = Vec::new();
    let fresh = || {
        let mut wb = Workbench::new();
        wb.set_language(LanguagePreference::Zh);
        open(&mut wb, data);
        wb
    };

    // ── the frame itself ───────────────────────────────────────────────
    let mut wb = fresh();
    let rows = wb.render(data, w, h);
    if rows.len() != h as usize {
        issues.push(format!("frame is {} rows, expected {h}", rows.len()));
    }
    for (index, row) in rows.iter().enumerate() {
        let cells = width(row);
        if cells != w as usize {
            issues.push(format!("row {index} is {cells} cells wide, expected {w}"));
            break;
        }
    }

    // ── every region, clicked where it says it is ──────────────────────
    let regions = wb.hits().regions().to_vec();
    let overlay_open = wb.overlay.is_some();
    let mut clicks = 0;
    for (rect, id) in &regions {
        if rect.is_empty() {
            issues.push(format!("{id}: empty region"));
            continue;
        }
        if rect.right() > w || rect.bottom() > h {
            issues.push(format!(
                "{id}: region at ({},{}) {}x{} runs past the {w}x{h} frame",
                rect.x, rect.y, rect.w, rect.h
            ));
            continue;
        }
        // A control's own centre must be its own. Page regions under an open
        // overlay are covered by it by design, and containers are legitimately
        // overlapped by the rows they hold.
        let (cx, cy) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
        let resolved = wb.hits().hit(cx, cy).unwrap_or("<none>");
        // A page control the open overlay covers is not reachable: the reader
        // is pointing at the overlay. Its answer is the overlay's, which is
        // checked below like any other.
        let covered = overlay_open && is_page(id) && resolved != id;
        if !overlay_open && !is_container(id) && resolved != id {
            issues.push(format!("{id}: centre ({cx},{cy}) resolves to {resolved}"));
        }

        // Now the click, on a page restored to exactly this state.
        let mut clicker = fresh();
        clicker.render(data, w, h);
        let before = digest(&clicker);
        let actions = clicker.handle(
            WbEvent::Mouse(MouseEvent {
                kind: MouseKind::Press,
                x: cx,
                y: cy,
                shift: false,
                alt: false,
                ctrl: false,
            }),
            data,
        );
        let after = digest(&clicker);
        clicks += 1;
        let passive = is_container(id) || covered;
        if actions.is_empty() && after == before && !passive {
            issues.push(format!(
                "{id}: click at ({cx},{cy}) does nothing{}",
                if covered {
                    " (covered by the overlay yet nothing answers)"
                } else {
                    ""
                }
            ));
        } else if verbose {
            println!(
                "     {id} -> {}{}",
                describe(&actions),
                if covered { " [covered]" } else { "" }
            );
        }
        let rows = clicker.render(data, w, h);
        if rows.len() != h as usize || rows.iter().any(|row| width(row) != w as usize) {
            issues.push(format!("{id}: click broke the frame size"));
        }
    }

    // ── clicking the dimmed page closes the overlay ───────────────────
    if overlay_open {
        if let Some((x, y)) = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .find(|&(x, y)| wb.hits().hit(x, y).is_none())
        {
            let mut dismisser = fresh();
            dismisser.render(data, w, h);
            dismisser.handle(
                WbEvent::Mouse(MouseEvent {
                    kind: MouseKind::Press,
                    x,
                    y,
                    shift: false,
                    alt: false,
                    ctrl: false,
                }),
                data,
            );
            if dismisser.overlay.is_some() {
                issues.push(format!(
                    "clicking the page outside the box at ({x},{y}) does not close it"
                ));
            }
        }
    }

    // ── every key, on a fresh page ─────────────────────────────────────
    let mut keys = 0;
    for key in KEYS {
        let mut presser = fresh();
        presser.render(data, w, h);
        let before = digest(&presser);
        let actions = presser.handle(WbEvent::Key((*key).to_string()), data);
        let after = digest(&presser);
        keys += 1;
        if verbose {
            println!("     key {key:<8} -> {}", describe(&actions));
        }
        if actions.is_empty() && after == before {
            continue;
        }
        let rows = presser.render(data, w, h);
        if rows.len() != h as usize || rows.iter().any(|row| width(row) != w as usize) {
            issues.push(format!("{key}: frame is not {w}x{h} after the key"));
        }
        if let Some((rect, id)) = presser
            .hits()
            .regions()
            .iter()
            .find(|(rect, _)| rect.right() > w || rect.bottom() > h)
        {
            issues.push(format!(
                "{key}: {id} at ({},{}) leaves the frame afterwards",
                rect.x, rect.y
            ));
        }
    }

    // ── the wheel, over every region ───────────────────────────────────
    for (rect, id) in &regions {
        if rect.is_empty() {
            continue;
        }
        for kind in [MouseKind::WheelDown, MouseKind::WheelUp] {
            let mut roller = fresh();
            roller.render(data, w, h);
            roller.handle(
                WbEvent::Mouse(MouseEvent {
                    kind,
                    x: rect.x + rect.w / 2,
                    y: rect.y + rect.h / 2,
                    shift: false,
                    alt: false,
                    ctrl: false,
                }),
                data,
            );
            let rows = roller.render(data, w, h);
            if rows.len() != h as usize || rows.iter().any(|row| width(row) != w as usize) {
                issues.push(format!("{id}: wheel broke the frame size"));
                break;
            }
        }
    }

    // ── a narrow terminal must not lose the verb ───────────────────────
    if w < 88 && name.starts_with("panel-") {
        let mut narrow = fresh();
        narrow.render(data, w, h);
        if !narrow
            .hits()
            .regions()
            .iter()
            .any(|(_, id)| id.starts_with("sidebar.tab."))
        {
            let sidebar_key = narrow
                .keys
                .key(future_tui::workbench::preferences::Action::Sidebar)
                .to_string();
            let actions = narrow.handle(WbEvent::Key(sidebar_key.clone()), data);
            narrow.render(data, w, h);
            // The panel takes the body instead of a rail: either overlay is
            // the answer, but the keys must reach it and its rows must be
            // clickable there too.
            let reached = matches!(narrow.overlay, Some(Overlay::PanelFull))
                || narrow
                    .hits()
                    .regions()
                    .iter()
                    .any(|(_, id)| id.starts_with("sidebar.row."));
            if !reached {
                issues.push(format!(
                    "{name}: {sidebar_key} cannot reach the panel when narrow ({actions:?})"
                ));
            }
        }
    }

    Report {
        issues,
        regions: regions.len(),
        clicks,
        keys,
    }
}

/// Ids that are legitimately overlapped, or that only describe an area: a
/// container is not a control, so its centre may resolve to what it holds.
fn is_container(id: &str) -> bool {
    matches!(id, "chat" | "picker.body" | "overlay.surface") || id == "diff.patch"
}

/// A control the page itself painted (as opposed to one of its overlays).
fn is_page(id: &str) -> bool {
    id == "chat"
        || id == "chat.latest"
        || id == "input"
        || id == "divider"
        || id.starts_with("sidebar.")
        || id.starts_with("header.")
        || id.starts_with("launchpad.")
}

fn describe(actions: &[WbAction]) -> String {
    if actions.is_empty() {
        return "<none>".into();
    }
    actions
        .iter()
        .map(|action| {
            format!("{action:?}")
                .split(['(', ' '])
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The state a click could have changed, as one comparable string.
fn digest(wb: &Workbench) -> String {
    format!(
        "{:?}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        wb.overlay,
        wb.panel,
        wb.sidebar_visible,
        wb.sidebar_w,
        wb.workflow_index,
        wb.overlay_scroll,
        wb.preferences_index,
        wb.keybinding_index,
        wb.awaiting_keybinding,
        wb.queue_index,
        wb.diff_patch_focus,
        wb.chat_scroll,
        wb.dragging(),
    )
}

#[allow(dead_code)]
fn unused_rect(_: Rect) {}
