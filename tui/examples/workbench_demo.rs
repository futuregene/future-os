//! Deterministic visual review frames. This uses the real workbench renderer,
//! state machine, i18n and hit maps; only the fixture data is synthetic.
use future_tui::utils::strip_ansi_codes;
use future_tui::workbench::demo;
use future_tui::workbench::i18n::{t, Key, LanguagePreference};
use future_tui::workbench::layout::{self, Rect};
use future_tui::workbench::mouse::{MouseEvent, MouseKind};
use future_tui::workbench::picker::PickerKind;
use future_tui::workbench::state::{
    ApprovalView, DiffView, ForkPoint, PanelId, QueuedPrompt, WorktreeView,
};
use future_tui::workbench::{Overlay, PaneId, WbAction, WbEvent, Workbench};

fn main() {
    let mut scenario = demo::Scenario::Launchpad;
    let mut width: u16 = 120;
    let mut height: u16 = 38;
    let mut plain = false;
    let mut self_test = false;
    let mut picker: Option<PickerKind> = None;
    let mut language = LanguagePreference::System;
    let mut overlay: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" | "-s" => match args.next().as_deref().and_then(demo::Scenario::parse) {
                Some(parsed) => scenario = parsed,
                None => fail("unknown scenario"),
            },
            "--width" | "-w" => width = parse_size(&mut args),
            "--height" | "-h" => height = parse_size(&mut args),
            "--plain" => plain = true,
            "--ansi" => plain = false,
            "--picker" | "-p" => match args.next().as_deref().and_then(PickerKind::from_command) {
                Some(parsed) => picker = Some(parsed),
                None => fail("expected model | theme | sessions"),
            },
            "--overlay" | "-o" => overlay = args.next(),
            "--lang" | "-l" => match args.next().as_deref() {
                Some(value) => language = LanguagePreference::from_wire(value),
                None => fail("expected system | zh | en"),
            },
            "--self-test" => self_test = true,
            other => fail(&format!("unknown argument {other}")),
        }
    }

    if self_test {
        match run_self_test() {
            Ok(()) => println!("self-test ok"),
            Err(reason) => fail(&format!("self-test failed: {reason}")),
        }
        return;
    }

    let mut data = demo::data(scenario);
    let mut workbench = Workbench::new();
    workbench.set_language(language);
    data.input.placeholder = t(workbench.lang, Key::InputHintLaunchpad).to_string();
    if let Some(name) = overlay.as_deref() {
        match name {
            "queue" => {
                data.queue = vec![
                    QueuedPrompt {
                        run_id: "run-2".into(),
                        text: "把回归测试拆成单元测试和集成测试，并报告覆盖率".into(),
                    },
                    QueuedPrompt {
                        run_id: "run-3".into(),
                        text: "顺便检查 Windows 下的路径".into(),
                    },
                ];
                workbench.surface_command("/queue", &data);
            }
            "approval" => {
                data.approval = Some(ApprovalView {
                    request_id: "approval-42".into(),
                    tool: "shell".into(),
                    kind: "command".into(),
                    risk: "HIGH".into(),
                    title: "执行测试与覆盖率检查".into(),
                    summary: "在工作区运行 Rust 测试并写入覆盖率报告".into(),
                    action: "{\n  \"command\": \"cargo test -p future-tui\"\n}".into(),
                    requested_action: "执行 shell 命令；可读取项目文件，写入 target/".into(),
                    sandbox_boundary: "workspace: D:/future-os\nwrite: target/**\nnetwork: denied"
                        .into(),
                    rules: vec![future_tui::workbench::state::AllowRule {
                        path: "target/**".into(),
                        access: "write".into(),
                    }],
                });
                workbench.overlay = Some(Overlay::Approval);
            }
            "diff" => {
                data.diff = Some(DiffView {
                    files: vec![
                        future_tui::workbench::diff_review::DiffFile { path: "tui/src/workbench/editor.rs".into(), status: "M".into() },
                        future_tui::workbench::diff_review::DiffFile { path: "tui/src/workbench/run.rs".into(), status: "M".into() },
                        future_tui::workbench::diff_review::DiffFile { path: "docs/guide/tui.zh-CN.md".into(), status: "M".into() },
                    ],
                    selected: 0,
                    patch: "diff --git a/tui/src/workbench/editor.rs b/tui/src/workbench/editor.rs\n@@ -18,6 +18,12 @@ impl EditorState {\n     pub fn key(&mut self, key: &str) {\n-        self.input.handle_key(key);\n+        match key {\n+            \"shift+enter\" | \"alt+enter\" => self.input.insert_text(\"\\n\"),\n+            other => self.input.handle_key(other),\n+        }\n     }\n }".into(),
                    error: None,
                });
                workbench.overlay = Some(Overlay::Diff);
            }
            "history" | "tree" | "fork" | "worktree" => {
                if name == "fork" {
                    data.fork_points = vec![
                        ForkPoint {
                            id: "e1".into(),
                            text: "先比较 DSH-TUI、Codex、OpenCode 与 Claude Code".into(),
                        },
                        ForkPoint {
                            id: "e7".into(),
                            text: "实现多行编辑器，再处理队列与滚动".into(),
                        },
                    ];
                }
                if name == "worktree" {
                    data.worktrees = vec![
                        WorktreeView {
                            path: "D:/future-os".into(),
                            label: "main".into(),
                            description: "主工作区".into(),
                            current: true,
                        },
                        WorktreeView {
                            path: "D:/future-os/.worktrees/tui-visual-audit".into(),
                            label: "feat/tui-visual-audit".into(),
                            description: "截图审查与体验优化".into(),
                            current: false,
                        },
                    ];
                }
                workbench.overlay = Some(match name {
                    "history" => Overlay::History,
                    "tree" => Overlay::Tree,
                    "fork" => Overlay::Fork,
                    _ => Overlay::Worktree,
                });
            }
            "recent" | "workspaces" => {
                workbench.panel = match name {
                    "workspaces" => PanelId::Workspaces,
                    _ => PanelId::Recent,
                };
            }
            "help" | "palette" | "preferences" => {
                workbench.surface_command(&format!("/{name}"), &data);
            }
            "keybindings" => {
                workbench.overlay = Some(Overlay::Keybindings);
            }
            _ => {
                let _ = workbench.surface_command(&format!("/{name}"), &data);
            }
        }
    }
    workbench.render(&data, width.max(1), height.max(1));
    if let Some(kind) = picker {
        workbench.open_picker(kind, &data);
    }
    let rows = workbench.render(&data, width.max(1), height.max(1));
    for row in rows {
        if plain {
            println!("{}", strip_ansi_codes(&row).trim_end());
        } else {
            println!("{row}\x1b[0m");
        }
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2)
}
fn parse_size(args: &mut impl Iterator<Item = String>) -> u16 {
    match args.next().and_then(|value| value.parse().ok()) {
        Some(size) => size,
        None => fail("expected a number"),
    }
}

fn run_self_test() -> Result<(), String> {
    for (width, height) in [(120u16, 38u16), (90, 24)] {
        let data = demo::chat();
        let mut workbench = Workbench::new();
        workbench.render(&data, width, height);
        let press = |x: u16, y: u16| MouseEvent {
            kind: MouseKind::Press,
            x,
            y,
            shift: false,
            alt: false,
            ctrl: false,
        };
        let region = |workbench: &Workbench, id: &str| -> Result<Rect, String> {
            workbench
                .hits()
                .regions()
                .iter()
                .find(|(_, hit)| hit == id)
                .map(|(rect, _)| *rect)
                .ok_or_else(|| format!("{width}x{height}: no {id} region"))
        };
        let tab = region(&workbench, "sidebar.tab.workspaces")?;
        if workbench.handle(WbEvent::Mouse(press(tab.x, tab.y)), &data)
            != vec![WbAction::SwitchPanel(PanelId::Workspaces)]
        {
            return Err("sidebar tab".into());
        }
        let input = region(&workbench, "input")?;
        if workbench.handle(WbEvent::Mouse(press(input.x + 1, input.y)), &data)
            != vec![WbAction::FocusPane(PaneId::Input)]
        {
            return Err("input click".into());
        }
        let divider = region(&workbench, "divider")?;
        workbench.handle(WbEvent::Mouse(press(divider.x, divider.y + 1)), &data);
        let drag = MouseEvent {
            kind: MouseKind::Drag,
            x: divider.x + 4,
            y: divider.y + 1,
            shift: false,
            alt: false,
            ctrl: false,
        };
        if !matches!(
            workbench.handle(WbEvent::Mouse(drag), &data).as_slice(),
            [WbAction::ResizeSidebar(_)]
        ) {
            return Err("divider drag".into());
        }
        let wheel = MouseEvent {
            kind: MouseKind::WheelDown,
            x: 4,
            y: layout::layout(width, height, layout::SIDEBAR_DEFAULT)
                .body
                .y
                + 2,
            shift: false,
            alt: false,
            ctrl: false,
        };
        if !matches!(
            workbench.handle(WbEvent::Mouse(wheel), &data).as_slice(),
            [WbAction::Scroll(_)]
        ) {
            return Err("wheel".into());
        }
        workbench.open_picker(PickerKind::Sessions, &data);
        workbench.render(&data, width, height);
        let row = region(&workbench, "picker.row.1")?;
        if !matches!(
            workbench
                .handle(WbEvent::Mouse(press(row.x + 1, row.y)), &data)
                .as_slice(),
            [WbAction::SwitchSession(_)]
        ) {
            return Err("picker click".into());
        }
    }
    Ok(())
}
