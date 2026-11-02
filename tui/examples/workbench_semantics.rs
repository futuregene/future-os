//! The behavioural half of the interaction audit: not "did something answer"
//! but "did the right thing answer".
//!
//! Each case drives the real workbench through a sequence a reader would
//! perform — scroll a long list, filter it, resize, tab between panes — and then
//! checks the *specific* answer: the session under the pointer, the file the
//! patch belongs to, the command the filter left visible.

use future_tui::workbench::demo;
use future_tui::workbench::diff_review::DiffFile;
use future_tui::workbench::i18n::LanguagePreference;
use future_tui::workbench::layout::Rect;
use future_tui::workbench::mouse::{MouseEvent, MouseKind};
use future_tui::workbench::picker::PickerKind;
use future_tui::workbench::state::{
    ApprovalView, DiffView, PanelId, QueuedPrompt, SessionRow, WorkbenchData,
};
use future_tui::workbench::{Overlay, WbAction, WbEvent, Workbench};

type Case = (&'static str, fn() -> Result<(), String>);

fn main() {
    let cases: &[Case] = &[
        (
            "session picker scrolls and clicks its own row",
            session_picker_row_after_scroll,
        ),
        (
            "session picker filter and click agree",
            session_picker_after_filter,
        ),
        ("fork click forks the message it drew", fork_row),
        ("worktree click selects the path it drew", worktree_row),
        ("queue click recalls the prompt it drew", queue_row),
        ("approval buttons send their own decision", approval_buttons),
        (
            "approval hides the rule button without a rule",
            approval_no_rule,
        ),
        ("diff click opens the file under the pointer", diff_row),
        ("diff wide terminal stays aligned", diff_wide),
        ("diff tab moves focus between panes", diff_focus),
        (
            "palette filter and click run the shown command",
            palette_row,
        ),
        (
            "preferences click selects the row under the pointer",
            preferences_row,
        ),
        ("preferences survive a short window", preferences_short),
        (
            "keybinding click starts capture on that action",
            keybinding_row,
        ),
        (
            "picker model click chooses the model under the pointer",
            picker_model_row,
        ),
        (
            "picker theme click applies the theme under the pointer",
            picker_theme_row,
        ),
        (
            "picker sessions click switches to that session",
            picker_session_row,
        ),
        (
            "wheel scrolls the chat and asks for history at the top",
            wheel_scroll,
        ),
        (
            "escape closes every overlay without deciding",
            escape_closes,
        ),
        (
            "clicking a sidebar tab switches and focuses it",
            sidebar_tab,
        ),
        (
            "clicking the divider starts a drag and does not jump",
            divider_drag,
        ),
        ("launchpad chips cycle their own field", launchpad_chip),
        (
            "a conversation link opens from the cell that draws it",
            chat_link_row,
        ),
        ("typing a draft keeps every character", typing),
    ];
    let mut failures = 0;
    for (name, case) in cases {
        match case() {
            Ok(()) => println!("ok   {name}"),
            Err(reason) => {
                failures += 1;
                println!("FAIL {name}\n       {reason}");
            }
        }
    }
    println!(
        "\n{}/{} behavioural cases pass",
        cases.len() - failures,
        cases.len()
    );
    if failures > 0 {
        std::process::exit(1);
    }
}

fn wb() -> Workbench {
    let mut wb = Workbench::new();
    wb.set_language(LanguagePreference::Zh);
    wb
}

fn press(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseKind::Press,
        x,
        y,
        shift: false,
        alt: false,
        ctrl: false,
    }
}

fn centre(rect: Rect) -> (u16, u16) {
    (rect.x + rect.w / 2, rect.y + rect.h / 2)
}

fn region(wb: &Workbench, id: &str) -> Result<Rect, String> {
    wb.hits()
        .regions()
        .iter()
        .find(|(_, known)| known == id)
        .map(|(rect, _)| *rect)
        .ok_or_else(|| format!("no region {id}"))
}

fn sessions(count: usize) -> WorkbenchData {
    let mut data = demo::chat();
    data.sessions = (0..count)
        .map(|index| SessionRow {
            id: format!("sess-{index:03}"),
            title: format!("session {index}"),
            workspace: format!("D:/w/{index}"),
            ..SessionRow::default()
        })
        .collect();
    data
}

/// A long list, scrolled to its end: the last visible row must open the session
/// that row draws — the classic off-by-one when the window and the hit map are
/// computed separately. The sessions picker (`ctrl+r`) is the door that lists
/// them, so the row under the pointer is its row.
fn session_picker_row_after_scroll() -> Result<(), String> {
    let data = sessions(24);
    let mut wb = wb();
    wb.open_picker(PickerKind::Sessions, &data);
    for _ in 0..40 {
        wb.handle(WbEvent::Key("down".into()), &data);
    }
    wb.render(&data, 80, 24);
    let last = wb
        .hits()
        .regions()
        .iter()
        .filter(|(_, id)| id.starts_with("picker.row."))
        .filter_map(|(rect, id)| {
            let index: usize = id.strip_prefix("picker.row.")?.parse().ok()?;
            Some((*rect, index))
        })
        .max_by_key(|(_, index)| *index)
        .ok_or("no visible picker rows")?;
    let (rect, index) = last;
    let (x, y) = centre(rect);
    let actions = wb.handle(WbEvent::Mouse(press(x, y)), &data);
    let expected = format!("sess-{index:03}");
    match actions.as_slice() {
        [WbAction::SwitchSession(id)] if *id == expected => Ok(()),
        other => Err(format!(
            "row {index} (y={y}) opened {other:?}, expected SwitchSession({expected})"
        )),
    }
}

fn session_picker_after_filter() -> Result<(), String> {
    let mut data = demo::chat();
    data.sessions = vec![
        SessionRow {
            id: "alpha".into(),
            title: "parser bug".into(),
            workspace: "D:/w/a".into(),
            ..SessionRow::default()
        },
        SessionRow {
            id: "beta".into(),
            title: "TUI layout".into(),
            workspace: "D:/w/b".into(),
            ..SessionRow::default()
        },
        SessionRow {
            id: "gamma".into(),
            title: "parser refactor".into(),
            workspace: "D:/w/c".into(),
            ..SessionRow::default()
        },
    ];
    let mut wb = wb();
    wb.open_picker(PickerKind::Sessions, &data);
    for ch in "pars".chars() {
        wb.handle(WbEvent::Key(ch.to_string()), &data);
    }
    // Two rows survive ("parser bug", "parser refactor"); the second must open
    // gamma, not beta: the click resolves against the filtered window.
    wb.handle(WbEvent::Key("down".into()), &data);
    wb.render(&data, 100, 30);
    let row = region(&wb, "picker.row.1")?;
    let (x, y) = centre(row);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::SwitchSession(id)] if id == "gamma" => Ok(()),
        other => Err(format!("filtered row 1 opened {other:?}, expected gamma")),
    }
}

fn fork_row() -> Result<(), String> {
    let mut data = demo::chat();
    data.fork_points = (0..6)
        .map(|index| future_tui::workbench::state::ForkPoint {
            id: format!("e{index}"),
            role: future_tui::workbench::state::ForkRole::User,
            text: format!("point {index}"),
        })
        .collect();
    let mut wb = wb();
    wb.overlay = Some(Overlay::Fork);
    wb.render(&data, 100, 30);
    let row = region(&wb, "workflow.row.3")?;
    let (x, y) = centre(row);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::ForkAt(id)] if id == "e3" => Ok(()),
        other => Err(format!("fork row 3 gave {other:?}, expected e3")),
    }
}

fn worktree_row() -> Result<(), String> {
    let mut data = demo::chat();
    data.worktrees = (0..5)
        .map(|index| future_tui::workbench::state::WorktreeView {
            path: format!("D:/w/{index}"),
            label: format!("wt-{index}"),
            description: String::new(),
            current: index == 0,
        })
        .collect();
    let mut wb = wb();
    wb.overlay = Some(Overlay::Worktree);
    wb.render(&data, 100, 30);
    let row = region(&wb, "workflow.row.2")?;
    let (x, y) = centre(row);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::SelectWorktree(path)] if path == "D:/w/2" => Ok(()),
        other => Err(format!("worktree row 2 gave {other:?}")),
    }
}

fn queue_row() -> Result<(), String> {
    let mut data = demo::chat();
    data.queue = vec![
        QueuedPrompt {
            run_id: "run-a".into(),
            text: "first".into(),
        },
        QueuedPrompt {
            run_id: "run-b".into(),
            text: "second".into(),
        },
    ];
    let mut wb = wb();
    wb.surface_command("/queue", &data);
    wb.render(&data, 100, 30);
    let row = region(&wb, "queue.row.1")?;
    let (x, y) = centre(row);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::RecallQueued(id)] if id == "run-b" => Ok(()),
        other => Err(format!("queue row 1 gave {other:?}, expected run-b")),
    }
}

fn approval(with_rule: bool) -> WorkbenchData {
    let mut data = demo::chat();
    data.approval = Some(ApprovalView {
        request_id: "req-1".into(),
        rules: if with_rule {
            vec![future_tui::workbench::state::AllowRule {
                path: "target/**".into(),
                access: "write".into(),
            }]
        } else {
            Vec::new()
        },
        ..ApprovalView::default()
    });
    data
}

fn approval_buttons() -> Result<(), String> {
    let data = approval(true);
    for (id, expected) in [
        ("approval.y", (true, false)),
        ("approval.a", (true, true)),
        ("approval.n", (false, false)),
    ] {
        let mut wb = wb();
        wb.overlay = Some(Overlay::Approval);
        wb.render(&data, 120, 38);
        let row = region(&wb, id)?;
        let (x, y) = centre(row);
        let actions = wb.handle(WbEvent::Mouse(press(x, y)), &data);
        match actions.as_slice() {
            [WbAction::ApprovalDecision { approved, rule }] if (*approved, *rule) == expected => {}
            other => return Err(format!("{id} gave {other:?}, expected {expected:?}")),
        }
        if wb.overlay.is_some() {
            return Err(format!("{id} left the card open"));
        }
    }
    Ok(())
}

fn approval_no_rule() -> Result<(), String> {
    let data = approval(false);
    let mut wb = wb();
    wb.overlay = Some(Overlay::Approval);
    wb.render(&data, 120, 38);
    if wb.hits().regions().iter().any(|(_, id)| id == "approval.a") {
        return Err("a rule button exists with no rule to add".into());
    }
    // `a` must not approve anything on that card either.
    let actions = wb.handle(WbEvent::Key("a".into()), &data);
    if !actions.is_empty() {
        return Err(format!("`a` answered {actions:?} with no rule available"));
    }
    Ok(())
}

fn diff_data(files: usize) -> WorkbenchData {
    let mut data = demo::chat();
    data.diff = Some(DiffView {
        files: (0..files)
            .map(|index| DiffFile {
                path: format!("src/file_{index}.rs"),
                status: "M".into(),
            })
            .collect(),
        selected: 0,
        patch: "+ a line\n+ another line".into(),
        error: None,
    });
    data
}

fn diff_row() -> Result<(), String> {
    let data = diff_data(6);
    let mut wb = wb();
    wb.overlay = Some(Overlay::Diff);
    wb.render(&data, 120, 38);
    let row = region(&wb, "diff.file.2")?;
    let (x, y) = centre(row);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::SelectDiff(2)] => Ok(()),
        other => Err(format!("diff file 2 gave {other:?}")),
    }
}

/// The bug this catches: a box painted at column 0 whose hit regions were
/// computed for a centred box — invisible at 120 columns, a whole margin out at
/// 160.
fn diff_wide() -> Result<(), String> {
    for width in [120u16, 160, 200] {
        let data = diff_data(6);
        let mut wb = wb();
        wb.overlay = Some(Overlay::Diff);
        let rows = wb.render(&data, width, 44);
        let row = region(&wb, "diff.file.1")?;
        let (x, y) = centre(row);
        let painted = future_tui::utils::strip_ansi_codes(&rows[y as usize]);
        let kind = painted
            .chars()
            .nth(x as usize)
            .ok_or_else(|| format!("{width}: no cell at ({x},{y})"))?;
        if !kind.is_whitespace() && kind != '│' && kind != '›' && kind != 'M' && kind != 'A' {
            return Err(format!(
                "{width}: clicking ({x},{y}) lands on {kind:?}, not the file row"
            ));
        }
        match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
            [WbAction::SelectDiff(1)] => {}
            other => return Err(format!("{width}: file 1 gave {other:?}")),
        }
    }
    Ok(())
}

fn diff_focus() -> Result<(), String> {
    let data = diff_data(4);
    let mut wb = wb();
    wb.overlay = Some(Overlay::Diff);
    wb.render(&data, 120, 38);
    // Files first, then the patch: one `tab`, and the arrows move the patch.
    wb.handle(WbEvent::Key("left".into()), &data);
    let start = wb.workflow_index;
    wb.handle(WbEvent::Key("tab".into()), &data);
    wb.handle(WbEvent::Key("down".into()), &data);
    if wb.overlay_scroll == 0 {
        return Err("down did not scroll the patch after tab".into());
    }
    if wb.workflow_index != start {
        return Err("scrolling the patch moved the file selection".into());
    }
    wb.handle(WbEvent::Key("tab".into()), &data);
    wb.handle(WbEvent::Key("down".into()), &data);
    if wb.workflow_index != start + 1 {
        return Err(format!(
            "down did not move the file selection after tab (index {}, expected {})",
            wb.workflow_index,
            start + 1
        ));
    }
    Ok(())
}

fn palette_row() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.overlay = Some(Overlay::Palette);
    for ch in "diff".chars() {
        wb.handle(WbEvent::Key(ch.to_string()), &data);
    }
    wb.render(&data, 120, 38);
    let shown = wb.palette_rows(&data);
    if shown.is_empty() {
        return Err("the filter left no commands".into());
    }
    let expected = shown[0].command();
    let row = region(&wb, "palette.row.0")?;
    let (x, y) = centre(row);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::Command(command)] if *command == expected => {
            if wb.overlay.is_some() {
                return Err("the palette stayed open after running a command".into());
            }
            Ok(())
        }
        other => Err(format!(
            "clicking the first entry gave {other:?}, expected {expected}"
        )),
    }
}

fn preferences_row() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.open_preferences();
    wb.render(&data, 120, 38);
    let row = region(&wb, "preferences.row.3")?;
    let (x, y) = centre(row);
    wb.handle(WbEvent::Mouse(press(x, y)), &data);
    if wb.preferences_index != 3 {
        return Err(format!(
            "clicking row 3 selected {} instead",
            wb.preferences_index
        ));
    }
    Ok(())
}

fn preferences_short() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.open_preferences();
    // Walk to the last row on a tiny window: the target must be inside the
    // frame and still select that row.
    for _ in 0..40 {
        wb.handle(WbEvent::Key("down".into()), &data);
    }
    wb.render(&data, 60, 14);
    let last = wb
        .hits()
        .regions()
        .iter()
        .filter(|(rect, _)| rect.right() <= 60 && rect.bottom() <= 14)
        .filter_map(|(rect, id)| {
            let index: usize = id.strip_prefix("preferences.row.")?.parse().ok()?;
            Some((*rect, index))
        })
        .max_by_key(|(_, index)| *index)
        .ok_or("no preference row is clickable in a 60x14 window")?;
    let (rect, index) = last;
    let (x, y) = centre(rect);
    wb.handle(WbEvent::Mouse(press(x, y)), &data);
    if wb.preferences_index != index {
        return Err(format!(
            "clicking row {index} at ({x},{y}) selected {}",
            wb.preferences_index
        ));
    }
    Ok(())
}

fn keybinding_row() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.open_preferences();
    wb.overlay = Some(Overlay::Keybindings);
    wb.render(&data, 120, 38);
    let row = region(&wb, "keybinding.row.5")?;
    let (x, y) = centre(row);
    wb.handle(WbEvent::Mouse(press(x, y)), &data);
    if wb.keybinding_index != 5 || !wb.awaiting_keybinding {
        return Err(format!(
            "row 5 gave index {} capture {}",
            wb.keybinding_index, wb.awaiting_keybinding
        ));
    }
    Ok(())
}

fn picker_model_row() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.open_picker(PickerKind::Model, &data);
    wb.render(&data, 120, 38);
    let row = region(&wb, "picker.row.1")?;
    let (x, y) = centre(row);
    let actions = wb.handle(WbEvent::Mouse(press(x, y)), &data);
    if actions.is_empty() {
        return Err("clicking a model row did nothing".into());
    }
    if wb.overlay.is_some() {
        return Err("the model picker stayed open".into());
    }
    Ok(())
}

fn picker_theme_row() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.open_picker(PickerKind::Theme, &data);
    wb.render(&data, 120, 38);
    let row = region(&wb, "picker.row.2")?;
    let (x, y) = centre(row);
    let actions = wb.handle(WbEvent::Mouse(press(x, y)), &data);
    match actions.as_slice() {
        [WbAction::SetTheme(id)] => {
            if wb.palette.id != id.as_str() {
                return Err(format!(
                    "SetTheme({id}) but the palette is {}",
                    wb.palette.id
                ));
            }
            Ok(())
        }
        other => Err(format!("theme row 2 gave {other:?}")),
    }
}

fn picker_session_row() -> Result<(), String> {
    let data = sessions(8);
    let mut wb = wb();
    wb.open_picker(PickerKind::Sessions, &data);
    wb.render(&data, 120, 38);
    let row = region(&wb, "picker.row.2")?;
    let (x, y) = centre(row);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::SwitchSession(_)] => Ok(()),
        other => Err(format!("session row 2 gave {other:?}")),
    }
}

fn wheel_scroll() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.render(&data, 120, 38);
    let chat = region(&wb, "chat")?;
    let (x, y) = centre(chat);
    let before = wb.chat_scroll;
    wb.handle(
        WbEvent::Mouse(MouseEvent {
            kind: MouseKind::WheelUp,
            x,
            y,
            shift: false,
            alt: false,
            ctrl: false,
        }),
        &data,
    );
    if wb.chat_scroll <= before {
        return Err("wheel up did not scroll the conversation".into());
    }
    Ok(())
}

fn escape_closes() -> Result<(), String> {
    for overlay in [
        Overlay::Help,
        Overlay::Palette,
        Overlay::Fork,
        Overlay::Worktree,
        Overlay::Diff,
        Overlay::Approval,
        Overlay::Queue,
    ] {
        let data = if overlay == Overlay::Approval {
            approval(true)
        } else {
            demo::chat()
        };
        let mut wb = wb();
        wb.overlay = Some(overlay);
        wb.render(&data, 120, 38);
        let actions = wb.handle(WbEvent::Key("escape".into()), &data);
        if wb.overlay.is_some() {
            return Err(format!("{overlay:?} stayed open on escape"));
        }
        if actions
            .iter()
            .any(|action| matches!(action, WbAction::ApprovalDecision { .. }))
        {
            return Err(format!("escape decided something on {overlay:?}"));
        }
    }
    Ok(())
}

fn sidebar_tab() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.render(&data, 120, 38);
    let tab = region(&wb, "sidebar.tab.workspaces")?;
    let (x, y) = centre(tab);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::SwitchPanel(PanelId::Workspaces)] => {
            if wb.panel != PanelId::Workspaces {
                return Err("the tab did not switch the panel".into());
            }
            Ok(())
        }
        other => Err(format!("workspaces tab gave {other:?}")),
    }
}

fn divider_drag() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.render(&data, 120, 38);
    let divider = region(&wb, "divider")?;
    let before = wb.sidebar_w;
    wb.handle(WbEvent::Mouse(press(divider.x, divider.y)), &data);
    if !wb.dragging() {
        return Err("pressing the divider started no drag".into());
    }
    let actions = wb.handle(
        WbEvent::Mouse(MouseEvent {
            kind: MouseKind::Drag,
            x: divider.x - 4,
            y: divider.y,
            shift: false,
            alt: false,
            ctrl: false,
        }),
        &data,
    );
    if !matches!(actions.as_slice(), [WbAction::ResizeSidebar(_)]) {
        return Err(format!("dragging gave {actions:?}"));
    }
    if wb.sidebar_w == before {
        return Err("the drag moved nothing".into());
    }
    Ok(())
}

fn launchpad_chip() -> Result<(), String> {
    let data = demo::data(demo::Scenario::Launchpad);
    let mut wb = wb();
    wb.render(&data, 120, 38);
    let chip = region(&wb, "launchpad.model")?;
    let (x, y) = centre(chip);
    let actions = wb.handle(WbEvent::Mouse(press(x, y)), &data);
    if actions.is_empty() {
        return Err("the model chip did nothing".into());
    }
    Ok(())
}

/// The cell under the pointer opens the link it is drawn in — and it is the
/// *second* one, so a click that landed on the row rather than the span would be
/// caught saying so.
fn chat_link_row() -> Result<(), String> {
    let mut data = demo::chat();
    let link = |href: &str, text: &str| format!("\x1b]8;;{href}\x1b\\{text}\x1b]8;;\x1b\\");
    data.messages = vec![format!(
        "{} then {}",
        link("https://example.com/a", "the site"),
        link("futureos://session/sess-002", "that chat"),
    )]
    .into();
    let mut wb = wb();
    wb.render(&data, 120, 38);
    let target = region(&wb, "chat.link.1")?;
    let (x, y) = centre(target);
    match wb.handle(WbEvent::Mouse(press(x, y)), &data).as_slice() {
        [WbAction::SwitchSession(id)] if id == "sess-002" => Ok(()),
        other => Err(format!("the click answered {other:?}")),
    }
}

fn typing() -> Result<(), String> {
    let data = demo::chat();
    let mut wb = wb();
    wb.render(&data, 120, 38);
    let input = region(&wb, "input")?;
    let (x, y) = centre(input);
    wb.handle(WbEvent::Mouse(press(x, y)), &data);
    let mut text = String::new();
    for ch in "界面 ok".chars() {
        for action in wb.handle(WbEvent::Key(ch.to_string()), &data) {
            if let WbAction::EditEditor(editor) = action {
                text = editor.text().to_string();
            }
        }
    }
    if text.is_empty() {
        return Err("typing produced no editor text".into());
    }
    Ok(())
}
