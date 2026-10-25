//! Print one workbench frame at any size — for screenshots, for grepping a
//! layout, and for the mouse self-test the contract asks every worker to run.
//!
//! `cargo run -p future-tui --example workbench_demo -- --scenario chat --width 120 --height 38 --plain`
//! `cargo run -p future-tui --example workbench_demo -- --scenario chat --lang en`
//! `cargo run -p future-tui --example workbench_demo -- --self-test`

use future_tui::utils::strip_ansi_codes;
use future_tui::workbench::demo;
use future_tui::workbench::i18n::{t, Key, LanguagePreference};
use future_tui::workbench::layout::{self, Rect};
use future_tui::workbench::mouse::{MouseEvent, MouseKind};
use future_tui::workbench::picker::PickerKind;
use future_tui::workbench::state::PanelId;
use future_tui::workbench::{PaneId, WbAction, WbEvent, Workbench};

fn main() {
    let mut scenario = demo::Scenario::Launchpad;
    let mut width: u16 = 120;
    let mut height: u16 = 38;
    let mut plain = false;
    let mut self_test = false;
    let mut picker: Option<PickerKind> = None;
    // What the chrome is drawn in; `system` is what `future tui` itself does.
    let mut language = LanguagePreference::System;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" | "-s" => match args.next().as_deref().and_then(demo::Scenario::parse) {
                Some(parsed) => scenario = parsed,
                None => {
                    eprintln!("unknown scenario");
                    std::process::exit(2);
                }
            },
            "--width" | "-w" => width = parse_size(&mut args),
            "--height" | "-h" => height = parse_size(&mut args),
            "--plain" => plain = true,
            "--ansi" => plain = false,
            // Show the frame with one of the three menus open — the design
            // review of a picker needs a frame, not an agent.
            "--picker" | "-p" => match args.next().as_deref().and_then(PickerKind::from_command) {
                Some(parsed) => picker = Some(parsed),
                None => {
                    eprintln!("expected model | theme | sessions");
                    std::process::exit(2);
                }
            },
            "--lang" | "-l" => match args.next().as_deref() {
                Some(value) => language = LanguagePreference::from_wire(value),
                None => {
                    eprintln!("expected system | zh | en");
                    std::process::exit(2);
                }
            },
            "--self-test" => self_test = true,
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    if self_test {
        match run_self_test() {
            Ok(()) => {
                println!("self-test ok");
            }
            Err(reason) => {
                eprintln!("self-test failed: {reason}");
                std::process::exit(1);
            }
        }
        return;
    }

    let mut data = demo::data(scenario);
    let mut workbench = Workbench::new();
    workbench.set_language(language);
    // The demo's own input hint is chrome, not content: it follows the language
    // the frame is drawn in, or an English screenshot argues with itself.
    data.input.placeholder = t(workbench.lang, Key::InputHintLaunchpad).to_string();
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

fn parse_size(args: &mut impl Iterator<Item = String>) -> u16 {
    match args.next().and_then(|value| value.parse().ok()) {
        Some(size) => size,
        None => {
            eprintln!("expected a number");
            std::process::exit(2);
        }
    }
}

/// The contract's acceptance check (§6): at two sizes, the regions the frame
/// painted answer with the actions the input means.
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

        // A sidebar tab switches the panel.
        let tab = region(&workbench, "sidebar.tab.usage")?;
        let actions = workbench.handle(WbEvent::Mouse(press(tab.x, tab.y)), &data);
        if actions != vec![WbAction::SwitchPanel(PanelId::Usage)] {
            return Err(format!("{width}x{height}: tab click gave {actions:?}"));
        }

        // Clicking the input asks for focus.
        let input = region(&workbench, "input")?;
        let actions = workbench.handle(WbEvent::Mouse(press(input.x + 1, input.y)), &data);
        if actions != vec![WbAction::FocusPane(PaneId::Input)] {
            return Err(format!("{width}x{height}: input click gave {actions:?}"));
        }

        // Dragging the divider resizes the sidebar.
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
        let actions = workbench.handle(WbEvent::Mouse(drag), &data);
        if !matches!(actions.as_slice(), [WbAction::ResizeSidebar(_)]) {
            return Err(format!("{width}x{height}: divider drag gave {actions:?}"));
        }

        // The wheel scrolls the conversation.
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
        let actions = workbench.handle(WbEvent::Mouse(wheel), &data);
        if !matches!(actions.as_slice(), [WbAction::Scroll(_)]) {
            return Err(format!("{width}x{height}: wheel gave {actions:?}"));
        }

        // A click on a picker row completes the choice, from the same hit map
        // the frame painted.
        workbench.open_picker(PickerKind::Sessions, &data);
        workbench.render(&data, width, height);
        let row = region(&workbench, "picker.row.1")?;
        let actions = workbench.handle(WbEvent::Mouse(press(row.x + 1, row.y)), &data);
        if !matches!(actions.as_slice(), [WbAction::SwitchSession(_)]) {
            return Err(format!("{width}x{height}: picker click gave {actions:?}"));
        }
    }
    Ok(())
}
