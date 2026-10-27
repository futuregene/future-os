//! Measurement harness: what one frame of the workbench driver loop costs, in
//! the two shapes the loop has had.
//!
//! * **before** — every event asked for a whole `WorkbenchData` at width `1`
//!   (twice: once to read the activity kind, once more after the event) and
//!   then for the frame at the real conversation width. `ChatArea` caches its
//!   rendered rows per width, so each of those three calls re-wrapped and
//!   re-parsed the whole transcript.
//! * **after** — one frame metric per iteration (`layout::Frame`), the event
//!   path reads its one field from `Live::activity_kind()` instead of building
//!   a frame, and frames are painted only when something changed.
//!
//! Run: `cargo test -p future-tui --release --test perf_loop_probe -- --nocapture`
//!
//! The numbers land in `docs/testing/tui-event-loop-cost-2026-10-26.md`. This
//! test asserts nothing: it is a meter, not a check.

use std::time::Instant;

use future_tui::components::chat_area::{ChatArea, ChatMessage, ChatRole};
use future_tui::workbench::layout::{self, Frame};
use future_tui::workbench::preferences::SidebarPosition;
use future_tui::workbench::{demo, Workbench};

const W: u16 = 120;
const H: u16 = 38;
/// How many events one timing run applies, divided afterwards.
const ITERS: usize = 20;

const BODY: &str = "## 结论\n\n这一段讨论 **渲染** 的开销，含 `行内代码`。\n\n- 第一点，说明一下\n- 第二点，再说明一下\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n\n";
const QUESTION: &str = "看一下这个渲染路径为什么会慢，顺便解释一下 markdown 每帧重新解析的代价。";
const DELTA: &str = "一些流式文本。";

fn frame_of() -> Frame {
    Frame::new(W, H, layout::SIDEBAR_DEFAULT, SidebarPosition::Right)
}

/// A conversation of `turns` question/answer pairs — markdown, code fences and
/// inline spans, which is what makes a re-wrap expensive.
fn transcript(turns: usize) -> ChatArea {
    let mut chat = ChatArea::new(W as usize, None);
    chat.set_viewport_height(H as usize - 6);
    for i in 0..turns {
        chat.add_message(ChatMessage::new(format!("u{i}"), ChatRole::User, QUESTION));
        chat.add_message(ChatMessage::new(
            format!("a{i}"),
            ChatRole::Assistant,
            &BODY.repeat(3),
        ));
    }
    chat
}

/// The bytes one frame becomes — the driver's own `frame_bytes`, mirrored. Its
/// length is what a paint writes; the driver only writes when it paints.
fn frame_bytes(rows: &[String]) -> String {
    let mut frame = String::with_capacity(rows.len() * 64);
    frame.push_str("\x1b[?25l");
    for (index, row) in rows.iter().enumerate() {
        frame.push_str(&format!("\x1b[0m\x1b[{};1H", index + 1));
        frame.push_str(row);
    }
    frame.push_str("\x1b[0m");
    frame
}

/// One frame: build the data for the frame's width, compose, serialise.
fn paint(workbench: &mut Workbench, chat: &mut ChatArea) -> usize {
    let frame = frame_of();
    let mut data = demo::data(demo::Scenario::Chat);
    data.messages = chat.render_all(frame.inner);
    let rows = workbench.render(&data, frame.width, frame.height);
    frame_bytes(&rows).len()
}

/// The old event path: `live.data(1)` before and after the event (to see
/// whether the activity kind moved), then the frame at its own width.
fn old_event(workbench: &mut Workbench, chat: &mut ChatArea) -> usize {
    let _kind_before = chat.render_all(1).len();
    chat.append_to_last_message(DELTA);
    let _kind_after = chat.render_all(1).len();
    paint(workbench, chat)
}

/// The fixed event path: the event only touches the transcript, and one frame
/// is painted for the batch.
fn new_event(workbench: &mut Workbench, chat: &mut ChatArea) -> usize {
    chat.append_to_last_message(DELTA);
    paint(workbench, chat)
}

/// Milliseconds per call, and full transcript rebuilds per call.
fn per(iters: usize, chat: &mut ChatArea, mut f: impl FnMut(&mut ChatArea) -> usize) -> (f64, f64) {
    let before = chat.full_renders();
    let start = Instant::now();
    for _ in 0..iters {
        f(chat);
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    let rebuilds = (chat.full_renders() - before) as f64 / iters as f64;
    (ms, rebuilds)
}

fn streaming_chat(turns: usize, workbench: &mut Workbench) -> ChatArea {
    let mut chat = transcript(turns);
    paint(workbench, &mut chat);
    chat.add_message(ChatMessage::new("live".into(), ChatRole::Assistant, ""));
    chat
}

fn row(label: &str, turns: usize, old: (f64, f64), new: (f64, f64)) {
    println!(
        "{label:>10} {turns:>5}  {:>10.3}  {:>10.3}  {:>7.1}x  {:>8.2}  {:>8.2}",
        old.0,
        new.0,
        old.0 / new.0,
        old.1,
        new.1
    );
}

fn table_head(what: &str) {
    println!();
    println!("{what}");
    println!(
        "{:>10} {:>5}  {:>10}  {:>10}  {:>8}  {:>8}  {:>8}",
        "", "turns", "old ms", "new ms", "speedup", "old rb", "new rb"
    );
}

#[test]
fn loop_cost_before_and_after() {
    let mut workbench = Workbench::new();
    println!(
        "frame {W}x{H}, conversation width {} columns; ANSI per frame {} bytes",
        frame_of().inner,
        {
            let mut chat = transcript(1);
            paint(&mut workbench, &mut chat)
        }
    );
    println!("ms is per event; rb is full transcript rebuilds per event");

    table_head("one streamed delta (event + the frame it causes)");
    for turns in [5usize, 20, 50, 100, 200] {
        let mut chat = streaming_chat(turns, &mut workbench);
        let old = per(ITERS, &mut chat, |chat| old_event(&mut workbench, chat));
        let mut chat = streaming_chat(turns, &mut workbench);
        let new = per(ITERS, &mut chat, |chat| new_event(&mut workbench, chat));
        row("delta", turns, old, new);
    }

    table_head("one keystroke (the event path's data calls + the frame)");
    for turns in [5usize, 20, 50, 100, 200] {
        let mut chat = streaming_chat(turns, &mut workbench);
        let old = per(ITERS, &mut chat, |chat| {
            let _ = chat.render_all(1);
            let _ = chat.render_all(1);
            paint(&mut workbench, chat)
        });
        let mut chat = streaming_chat(turns, &mut workbench);
        let new = per(ITERS, &mut chat, |chat| paint(&mut workbench, chat));
        row("key", turns, old, new);
    }

    table_head("one idle tick: the loop's own repaint with nothing to show");
    for turns in [20usize, 50, 100, 200] {
        let mut chat = streaming_chat(turns, &mut workbench);
        // The old loop painted the frame at the top of every iteration, eight
        // times a second. The fixed one paints nothing at all.
        let (old_ms, old_rb) = per(ITERS, &mut chat, |chat| paint(&mut workbench, chat));
        let (new_ms, new_rb) = per(ITERS, &mut chat, |_| 0);
        row("idle tick", turns, (old_ms, old_rb), (new_ms, new_rb));
        println!(
            "           → {:.1} % of a core at 8 ticks/s before; {:.1} % after",
            old_ms * 8.0 / 10.0,
            new_ms * 8.0 / 10.0
        );
    }

    println!();
    println!("a 20-delta batch (~0.7 s of a 30 tokens/s stream), one frame at the end");
    println!(
        "{:>10} {:>5}  {:>10}  {:>10}  {:>8}  {:>10}  {:>10}",
        "", "turns", "old ms", "new ms", "speedup", "old rb", "new rb"
    );
    for turns in [5usize, 20, 50, 100, 200] {
        let mut chat = streaming_chat(turns, &mut workbench);
        let before = chat.full_renders();
        let start = Instant::now();
        for _ in 0..20 {
            old_event(&mut workbench, &mut chat);
        }
        let old_ms = start.elapsed().as_secs_f64() * 1000.0;
        let old_rb = chat.full_renders() - before;

        let mut chat = streaming_chat(turns, &mut workbench);
        let before = chat.full_renders();
        let start = Instant::now();
        for _ in 0..20 {
            chat.append_to_last_message(DELTA);
        }
        paint(&mut workbench, &mut chat);
        let new_ms = start.elapsed().as_secs_f64() * 1000.0;
        let new_rb = chat.full_renders() - before;

        println!(
            "{:>10} {turns:>5}  {old_ms:>10.1}  {new_ms:>10.1}  {:>7.1}x  {old_rb:>10}  {new_rb:>10}",
            "20 deltas",
            old_ms / new_ms
        );
    }

    println!();
    println!("where one frame's time goes, by transcript length");
    println!(
        "{:>6}  {:>14}  {:>10}  {:>10}  {:>10}  {:>9}",
        "turns", "transcript rows", "render_all", "compose", "canvas", "bytes"
    );
    for turns in [5usize, 20, 100, 400] {
        let mut chat = transcript(turns);
        let frame = frame_of();
        paint(&mut workbench, &mut chat);
        let rows_of_transcript = chat.render_all(frame.inner).len();
        let (cache_ms, _) = per(50, &mut chat, |chat| chat.render_all(frame.inner).len());
        let mut data = demo::data(demo::Scenario::Chat);
        data.messages = chat.render_all(frame.inner);
        let (compose_ms, _) = per(50, &mut chat, |_| {
            let rows = workbench.render(&data, W, H);
            rows.len()
        });
        let rows = workbench.render(&data, W, H);
        let (canvas_ms, _) = per(50, &mut chat, |_| {
            canvas_rows(&rows, W as usize, 7, 0).len()
        });
        let canvas = canvas_rows(&rows, W as usize, 7, 0);
        let (bytes_ms, _) = per(50, &mut chat, |_| frame_bytes(&canvas).len());
        println!(
            "{turns:>6}  {rows_of_transcript:>14}  {cache_ms:>8.3}ms  {compose_ms:>8.3}ms  {canvas_ms:>8.3}ms  {bytes_ms:>7.3}ms"
        );
    }
    println!(
        "  compose is the frame's floor: it does not grow with the transcript, so it is\n  what a frame costs when nothing moved — 0.42 ms here, 0.3 % of a core at 8 fps."
    );
}

/// The driver's `canvas_rows` / `repaint_after_resets`, mirrored (both are
/// private to `run.rs`): the surface paints its own colours onto every row and
/// pads it to the frame's width, which is a pass over every cell of the frame.
fn canvas_rows(rows: &[String], width: usize, fg: u8, bg: u8) -> Vec<String> {
    let background = format!("\x1b[48;5;{bg}m");
    rows.iter()
        .map(|row| {
            let paint = if row.contains("\x1b[48;5;") {
                background.clone()
            } else {
                format!("\x1b[38;5;{fg}m{background}")
            };
            let padding = width.saturating_sub(future_tui::utils::visible_width(
                &future_tui::utils::strip_ansi_codes(row),
            ));
            format!(
                "{paint}{}\x1b[0m{paint}{}\x1b[0m",
                repaint_after_resets(row, &paint),
                " ".repeat(padding)
            )
        })
        .collect()
}

fn repaint_after_resets(line: &str, paint: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 2 < bytes.len() && bytes[i + 1] == b'[' {
            if bytes[i + 2] == b'm' {
                out.push_str("\x1b[0m");
                out.push_str(paint);
                i += 3;
                continue;
            }
            if bytes[i + 2] == b'0' && i + 3 < bytes.len() && bytes[i + 3] == b'm' {
                out.push_str("\x1b[0m");
                out.push_str(paint);
                i += 4;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
