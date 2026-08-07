//! Render-parity driver (Rust side).
//!
//! Twin of `tui/render-parity.ts`: reads the shared corpus
//! (`tui/tests/parity-corpus.json`) and renders every case with the
//! Rust implementation — MarkdownRenderer, ChatArea (including the
//! streaming prefix-cache path) and the terminal-image helpers — printing
//! one line per case:
//!
//!     <kind>|<name>|<base64(JSON.stringify(result))>
//!
//! The TS twin was retired; the harness (`tui/tests/diff-ts-rust.sh`) byte-compares
//! byte-compares the two outputs. serde_json escaping matches JS
//! `JSON.stringify` exactly (verified), and the standard base64 alphabet
//! with padding matches `Buffer.toString("base64")`.
//!
//! Usage: cargo run -p tui-rust --example render_parity -- <corpus.json>

use base64::Engine as _;
use serde_json::{json, Value};

use future_tui::components::chat_area::{ChatArea, ChatMessage, ChatRole, RunState, ToolStatus};
use future_tui::components::markdown::{DefaultTextStyle, MarkdownRenderer, MarkdownThemePartial};
use future_tui::terminal_image::{
    calculate_image_rows, collect_kitty_image_ids, delete_all_kitty_images, delete_kitty_image,
    delete_kitty_images, encode_iterm2, encode_kitty, extract_kitty_image_ids, get_gif_dimensions,
    get_image_dimensions, get_jpeg_dimensions, get_png_dimensions, get_webp_dimensions, hyperlink,
    image_fallback, is_image_line, render_image, set_capabilities, CellDimensions, ImageDimensions,
    ImageProtocol, ImageRenderOptions, Iterm2EncodeOptions, KittyEncodeOptions, RenderImageResult,
    TerminalCapabilities,
};
use future_tui::theme::{dim as theme_dim, fg as theme_fg, italic as theme_italic};

fn main() {
    let corpus_path = std::env::args()
        .nth(1)
        .expect("usage: render_parity <corpus.json>");
    let corpus: Value =
        serde_json::from_str(&std::fs::read_to_string(&corpus_path).unwrap()).expect("corpus JSON");
    let mut out: Vec<String> = Vec::new();

    // ─── Markdown ────────────────────────────────────────────────────────
    if let Some(cases) = corpus.get("markdown").and_then(Value::as_array) {
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let text = c["text"].as_str().unwrap();
            let width = c["width"].as_u64().unwrap() as usize;

            // thinkingTheme: every accent color → thinking gray (244).
            let thinking_partial = MarkdownThemePartial {
                heading: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                link: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                link_url: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                code: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                code_block: Some(std::rc::Rc::new(|t: &str| theme_fg(244, &theme_dim(t)))),
                code_block_border: Some(std::rc::Rc::new(|t: &str| theme_fg(244, &theme_dim(t)))),
                quote: Some(std::rc::Rc::new(|t: &str| theme_fg(244, &theme_italic(t)))),
                quote_border: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                hr: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                list_bullet: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                strikethrough: Some(std::rc::Rc::new(|t: &str| theme_fg(244, t))),
                ..Default::default()
            };

            let theme = if c.get("theme").and_then(Value::as_str) == Some("thinking") {
                thinking_partial
            } else {
                MarkdownThemePartial::default()
            };

            // fgFn(spec): "fg124" → (t) => fg(124, t)
            let style = c.get("opts").and_then(|o| o.get("defaultStyle")).map(|ds| {
                let color = ds.get("color").and_then(Value::as_str).map(|spec| {
                    let n = spec
                        .strip_prefix("fg")
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(0);
                    let f: future_tui::components::markdown::StyleFn =
                        std::rc::Rc::new(move |t: &str| theme_fg(n, t));
                    f
                });
                DefaultTextStyle {
                    color,
                    bg_color: None,
                    bold: ds.get("bold").and_then(Value::as_bool).unwrap_or(false),
                    italic: ds.get("italic").and_then(Value::as_bool).unwrap_or(false),
                    strikethrough: false,
                    underline: false,
                }
            });

            let mut md = MarkdownRenderer::with_theme_and_style(theme, style);
            if let Some(px) = c
                .get("opts")
                .and_then(|o| o.get("paddingX"))
                .and_then(Value::as_u64)
            {
                let py = c["opts"]
                    .get("paddingY")
                    .and_then(Value::as_u64)
                    .map(|y| y as usize);
                md.set_padding(px as usize, py);
            }
            let lines = md.render_text(text, width);
            emit(
                &mut out,
                "markdown",
                name,
                Value::Array(lines.into_iter().map(Value::String).collect()),
            );
        }
    }

    // ─── ChatArea ────────────────────────────────────────────────────────
    if let Some(cases) = corpus.get("chat").and_then(Value::as_array) {
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let width = c["width"].as_u64().unwrap() as usize;
            let mut chat = ChatArea::new(width, None);
            chat.render(width);
            for m in c["msgs"].as_array().unwrap() {
                chat.add_message(parse_message(m));
            }
            if c.get("thinkingHidden").and_then(Value::as_bool) == Some(true) {
                chat.set_thinking_hidden(true);
            }
            let viewport = c
                .get("viewportHeight")
                .and_then(Value::as_u64)
                .map(|h| h as usize);
            if let Some(h) = viewport {
                chat.set_viewport_height(h);
            }
            let lines = match viewport {
                Some(_) => chat.render(width),
                None => chat.render_all(width),
            };
            emit(
                &mut out,
                "chat",
                name,
                Value::Array(lines.into_iter().map(Value::String).collect()),
            );
        }
    }

    // ─── ChatArea streaming (prefix-cache path, one frame per delta) ─────
    if let Some(cases) = corpus.get("chatStream").and_then(Value::as_array) {
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let width = c["width"].as_u64().unwrap() as usize;
            let mut chat = ChatArea::new(width, None);
            chat.render(width);
            let mut m = ChatMessage::new("m".to_string(), ChatRole::Assistant, "");
            m.pending = true;
            chat.add_message(m);
            let mut frames: Vec<Value> = Vec::new();
            for delta in c["deltas"].as_array().unwrap() {
                chat.append_to_last_message(delta.as_str().unwrap());
                let lines = chat.render_all(width);
                frames.push(Value::Array(lines.into_iter().map(Value::String).collect()));
            }
            emit(&mut out, "chatStream", name, Value::Array(frames));
        }
    }

    // ─── terminal-image ──────────────────────────────────────────────────
    if let Some(cases) = corpus.get("image").and_then(Value::as_array) {
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let args = c["args"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
            let result = dispatch_image(c["fn"].as_str().unwrap(), args);
            emit(&mut out, "image", name, result);
        }
    }

    println!("{}", out.join("\n"));
}

fn parse_message(m: &Value) -> ChatMessage {
    let role = match m["role"].as_str().unwrap() {
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        "system" => ChatRole::System,
        "tool" => ChatRole::Tool,
        r => panic!("unknown role {r}"),
    };
    let mut msg = ChatMessage::new(
        m["id"].as_str().unwrap().to_string(),
        role,
        m.get("content").and_then(Value::as_str).unwrap_or(""),
    );
    msg.name = m.get("name").and_then(Value::as_str).map(String::from);
    msg.tool = m.get("tool").and_then(Value::as_str).map(String::from);
    msg.tool_args = m.get("toolArgs").and_then(Value::as_str).map(String::from);
    msg.tool_status = m
        .get("toolStatus")
        .and_then(Value::as_str)
        .map(|s| match s {
            "running" => ToolStatus::Running,
            "complete" => ToolStatus::Complete,
            "error" => ToolStatus::Error,
            other => panic!("unknown toolStatus {other}"),
        });
    msg.exit_code = m.get("exitCode").and_then(Value::as_i64).map(|v| v as i32);
    msg.timestamp = m.get("timestamp").and_then(Value::as_u64);
    msg.thinking = m.get("thinking").and_then(Value::as_str).map(String::from);
    msg.pending = m.get("pending").and_then(Value::as_bool).unwrap_or(false);
    msg.stopped = m.get("stopped").and_then(Value::as_bool).unwrap_or(false);
    msg.welcome = m.get("welcome").and_then(Value::as_bool).unwrap_or(false);
    msg.run_id = m.get("runId").and_then(Value::as_str).map(String::from);
    msg.run_state = m.get("runState").and_then(Value::as_str).map(|s| match s {
        "queued" => RunState::Queued,
        "running" => RunState::Running,
        "terminal" => RunState::Terminal,
        "failed" => RunState::Failed,
        "cancelled" => RunState::Cancelled,
        "superseded" => RunState::Superseded,
        "lost_on_agent_restart" => RunState::LostOnAgentRestart,
        other => panic!("unknown runState {other}"),
    });
    msg.queue_position = m
        .get("queuePosition")
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    msg
}

fn dims(v: &Value) -> ImageDimensions {
    ImageDimensions {
        width_px: v["widthPx"].as_u64().unwrap() as usize,
        height_px: v["heightPx"].as_u64().unwrap() as usize,
    }
}

fn to_value_str(s: &str) -> Value {
    Value::String(s.to_string())
}

fn image_dims_value(d: ImageDimensions) -> Value {
    json!({ "widthPx": d.width_px, "heightPx": d.height_px })
}

fn render_result_value(r: RenderImageResult) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("sequence".to_string(), Value::String(r.sequence));
    map.insert("rows".to_string(), json!(r.rows));
    if let Some(id) = r.image_id {
        map.insert("imageId".to_string(), json!(id));
    }
    Value::Object(map)
}

fn dispatch_image(fn_name: &str, args: &[Value]) -> Value {
    match fn_name {
        "encodeKitty" => {
            let opts = &args[1];
            to_value_str(&encode_kitty(
                args[0].as_str().unwrap(),
                &KittyEncodeOptions {
                    columns: opts
                        .get("columns")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize),
                    rows: opts.get("rows").and_then(Value::as_u64).map(|v| v as usize),
                    image_id: opts
                        .get("imageId")
                        .and_then(Value::as_u64)
                        .map(|v| v as u32),
                    move_cursor: opts.get("moveCursor").and_then(Value::as_bool),
                },
            ))
        }
        "deleteKittyImage" => to_value_str(&delete_kitty_image(args[0].as_u64().unwrap() as u32)),
        "deleteAllKittyImages" => to_value_str(&delete_all_kitty_images()),
        "encodeITerm2" => {
            let opts = &args[1];
            to_value_str(&encode_iterm2(
                args[0].as_str().unwrap(),
                &Iterm2EncodeOptions {
                    width: opts.get("width").map(js_value_to_string),
                    height: opts.get("height").map(js_value_to_string),
                    name: opts.get("name").and_then(Value::as_str).map(String::from),
                    preserve_aspect_ratio: opts.get("preserveAspectRatio").and_then(Value::as_bool),
                    inline: opts.get("inline").and_then(Value::as_bool),
                },
            ))
        }
        "calculateImageRows" => json!(calculate_image_rows(
            dims(&args[0]),
            args[1].as_u64().unwrap() as usize,
            CellDimensions {
                width_px: args[2]["widthPx"].as_u64().unwrap() as usize,
                height_px: args[2]["heightPx"].as_u64().unwrap() as usize,
            },
        )),
        "getPngDimensions" => match get_png_dimensions(args[0].as_str().unwrap()) {
            Some(d) => image_dims_value(d),
            None => Value::Null,
        },
        "getJpegDimensions" => match get_jpeg_dimensions(args[0].as_str().unwrap()) {
            Some(d) => image_dims_value(d),
            None => Value::Null,
        },
        "getGifDimensions" => match get_gif_dimensions(args[0].as_str().unwrap()) {
            Some(d) => image_dims_value(d),
            None => Value::Null,
        },
        "getWebpDimensions" => match get_webp_dimensions(args[0].as_str().unwrap()) {
            Some(d) => image_dims_value(d),
            None => Value::Null,
        },
        "getImageDimensions" => {
            match get_image_dimensions(args[0].as_str().unwrap(), args[1].as_str().unwrap()) {
                Some(d) => image_dims_value(d),
                None => Value::Null,
            }
        }
        "isImageLine" => json!(is_image_line(args[0].as_str().unwrap())),
        "extractKittyImageIds" => Value::Array(
            extract_kitty_image_ids(args[0].as_str().unwrap())
                .into_iter()
                .map(|id| json!(id))
                .collect(),
        ),
        "collectKittyImageIds" => {
            let lines: Vec<String> = args[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l.as_str().unwrap().to_string())
                .collect();
            Value::Array(
                collect_kitty_image_ids(&lines)
                    .into_iter()
                    .map(|id| json!(id))
                    .collect(),
            )
        }
        "deleteKittyImages" => {
            let ids: std::collections::BTreeSet<u32> = args[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect();
            to_value_str(&delete_kitty_images(&ids))
        }
        "hyperlink" => to_value_str(&hyperlink(
            args[0].as_str().unwrap(),
            args[1].as_str().unwrap(),
        )),
        "imageFallback" => {
            let dims_arg = args.get(1).map(dims);
            to_value_str(&image_fallback(
                args[0].as_str().unwrap(),
                dims_arg,
                args.get(2).and_then(Value::as_str),
            ))
        }
        "renderImage" => {
            let caps = &args[0];
            set_capabilities(TerminalCapabilities {
                images: match caps.get("images") {
                    Some(Value::String(s)) if s == "kitty" => ImageProtocol::Kitty,
                    Some(Value::String(s)) if s == "iterm2" => ImageProtocol::Iterm2,
                    _ => ImageProtocol::None,
                },
                true_color: caps
                    .get("trueColor")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                hyperlinks: caps
                    .get("hyperlinks")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
            let opts = args.get(3).cloned().unwrap_or(json!({}));
            match render_image(
                args[1].as_str().unwrap(),
                dims(&args[2]),
                &ImageRenderOptions {
                    max_width_cells: opts
                        .get("maxWidthCells")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize),
                    max_height_cells: opts
                        .get("maxHeightCells")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize),
                    preserve_aspect_ratio: opts.get("preserveAspectRatio").and_then(Value::as_bool),
                    image_id: opts
                        .get("imageId")
                        .and_then(Value::as_u64)
                        .map(|v| v as u32),
                    move_cursor: opts.get("moveCursor").and_then(Value::as_bool),
                },
            ) {
                Some(r) => render_result_value(r),
                None => Value::Null,
            }
        }
        other => panic!("unknown image fn: {other}"),
    }
}

/// JS template-literal semantics for `width`/`height` options: numbers
/// stringify without a decimal point.
fn js_value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(f) = n.as_f64() {
                f.to_string()
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

fn emit(out: &mut Vec<String>, kind: &str, name: &str, value: Value) {
    let json = serde_json::to_string(&value).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
    out.push(format!("{kind}|{name}|{b64}"));
}
