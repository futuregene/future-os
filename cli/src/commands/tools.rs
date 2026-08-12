//! `future tools` — port of `cli/src/commands/tools.ts` (P2: full command
//! bodies for list/describe/call, including the browser tool surface).
//!
//! Remote tools are called over the MCP HTTP protocol (`commands/mcp.rs`).
//! The browser tool (`browser start`/`status`) lives in `browser_tools.rs`;
//! its session-based commands are a documented P3 gap.

use crate::commands::browser_tools::{browser_tool_catalog, call_browser_tool, is_browser_tool};
use crate::commands::mcp::{initialize_session, mcp_post, mcp_url, result_of};
use crate::constants::{auth_file, FUTURE_AUTH_PROVIDER};
use crate::output::Output;
use crate::utils::object::is_record;
use base64::Engine;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

// ── Catalog ─────────────────────────────────────────────────────────────────

/// One `ToolEntry` from tools.ts.
pub struct ToolEntry {
    pub description: &'static str,
    /// Ordered `{argName: description}` pairs (Object.entries order).
    pub args: Vec<(&'static str, &'static str)>,
    pub example: &'static str,
    pub input_required: bool,
    pub mask_supported: bool,
    pub output_supported: bool,
}

/// `TOOL_CATALOG` — `{...BROWSER_TOOL_CATALOG, ...remote tools}`, built once
/// and cached (the entries hold `&'static str` fields; the Vecs are heap).
fn tool_catalog() -> &'static Vec<(&'static str, ToolEntry)> {
    static CATALOG: std::sync::OnceLock<Vec<(&'static str, ToolEntry)>> =
        std::sync::OnceLock::new();
    CATALOG.get_or_init(build_tool_catalog)
}

fn build_tool_catalog() -> Vec<(&'static str, ToolEntry)> {
    let mut out = Vec::new();
    for (name, entry) in browser_tool_catalog() {
        out.push((
            name,
            ToolEntry {
                description: entry.description,
                args: entry.args.iter().map(|(k, v)| (*k, *v)).collect(),
                example: entry.example,
                input_required: false,
                mask_supported: false,
                output_supported: false,
            },
        ));
    }
    out.push((
        "search_paper",
        ToolEntry {
            description: "Search academic papers and extract requested information.",
            args: vec![
                ("queries", "search terms, one per query (required)"),
                ("information_to_extract", "what information to extract from the results (optional)"),
                ("max_results_per_query", "max papers to return per query, 1-20 (optional, default: 10)"),
            ],
            example: "{\"queries\": [\"CRISPR gene editing overview\", \"CRISPR applications 2025\"], \"information_to_extract\": \"key methods and recent advances\", \"max_results_per_query\": 8}",
            input_required: false,
            mask_supported: false,
            output_supported: false,
        },
    ));
    out.push((
        "get_paper",
        ToolEntry {
            description: "Get full paper content by identifier (PMID, DOI). Returns metadata (title, authors, journal, year, DOI) and complete body_text.",
            args: vec![
                ("paper_id", "paper identifier like \"PMID:12345678\" or \"DOI:10.xxx/...\" (required)"),
                ("max_k", "max result chunks to return (optional)"),
            ],
            example: "{\"paper_id\": \"PMID:12345678\", \"max_k\": 3}",
            input_required: false,
            mask_supported: false,
            output_supported: false,
        },
    ));
    out.push((
        "image_gen",
        ToolEntry {
            description: "Generate images from a text prompt.",
            args: vec![
                ("prompt", "description of the image to generate (required)"),
                ("size", "output dimensions, e.g. \"1024x1024\", \"1792x1024\" (optional, default: 1024x1024)"),
                ("quality", "image quality: \"low\", \"medium\", or \"high\" (optional, default: medium)"),
                ("n", "number of images to generate, 1–10 (optional, default: 1)"),
                ("output_format", "file format: only \"png\" is reliable (optional, default: png; jpg/webp are unstable)"),
            ],
            example: "{\"prompt\": \"A red fox in an autumn forest, golden hour\", \"size\": \"1024x1024\", \"n\": 1}",
            input_required: false,
            mask_supported: false,
            output_supported: true,
        },
    ));
    out.push((
        "image_edit",
        ToolEntry {
            description: "Edit an existing image using a text prompt. Requires --input <path> for the source image. Optional --mask <path> to limit edits to a region.",
            args: vec![
                ("prompt", "description of the desired edits (required)"),
                ("size", "output dimensions, e.g. \"1024x1024\" (optional)"),
                ("quality", "\"low\", \"medium\", or \"high\" (optional)"),
            ],
            example: "{\"prompt\": \"Convert to watercolor painting\"}",
            input_required: true,
            mask_supported: true,
            output_supported: true,
        },
    ));
    out.push((
        "read_image",
        ToolEntry {
            description: "Analyze an image: OCR text extraction, object recognition, visual Q&A. Requires --input <path> for the image file.",
            args: vec![
                ("question", "what to ask about the image, e.g. \"What text is in this image?\" or \"Describe this image\" (required)"),
                ("mime_type", "image MIME type (optional, default: image/png)"),
                ("max_tokens", "max tokens in the response (optional, default: 2000)"),
            ],
            example: "{\"question\": \"What text is in this image?\"}",
            input_required: true,
            mask_supported: false,
            output_supported: false,
        },
    ));
    out.push((
        "parse_doc",
        ToolEntry {
            description: "Parse a PDF or Word document into markdown, preserving text, tables, and formulas. Requires --input <path> for the document.",
            args: vec![
                ("file_type", "document type: \"pdf\" or \"docx\" (optional, default: pdf)"),
            ],
            example: "{\"file_type\": \"pdf\"}",
            input_required: true,
            mask_supported: false,
            output_supported: false,
        },
    ));
    out.push((
        "web_search",
        ToolEntry {
            description: "Search the web. Returns result titles, URLs, and snippets.",
            args: vec![
                ("query", "the search query string (required)"),
                (
                    "count",
                    "number of results to return, max 50 (optional, default: 10)",
                ),
            ],
            example: "{\"query\": \"BRCA1 variant classification guidelines 2025\", \"count\": 5}",
            input_required: false,
            mask_supported: false,
            output_supported: false,
        },
    ));
    out.push((
        "fetch_url",
        ToolEntry {
            description: "Fetch and extract the main content from a web page. Returns page title and clean text.",
            args: vec![
                ("url", "the full URL to fetch, e.g. https://example.com/article (required)"),
            ],
            example: "{\"url\": \"https://en.wikipedia.org/wiki/BRCA1\"}",
            input_required: false,
            mask_supported: false,
            output_supported: false,
        },
    ));
    out
}

fn find_tool_entry(name: &str) -> Option<&'static ToolEntry> {
    tool_catalog()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, e)| e)
}

// ── Error translation ───────────────────────────────────────────────────────

struct ErrorTranslation {
    description: &'static str,
    action: &'static str,
    retryable: bool,
}

/// `ERROR_TRANSLATIONS` — `key = "{toolName}|{pattern}"`, `_default` fallback.
const ERROR_TRANSLATIONS: &[(&str, &str, &str, bool)] = &[
    // ── image_gen ──────────────────────────────────────────────────────
    (
        "image_gen|azure_image_transport_failed",
        "Image generation transport error (remote renderer failure)",
        "Retry with --quality 'medium' or 'low'. 'high' quality is unstable — avoid it.",
        true,
    ),
    (
        "image_gen|insufficient_credit",
        "Account balance too low",
        "Top up your account and retry. Run 'future account balance' to check.",
        false,
    ),
    (
        "image_gen|This operation was aborted",
        "Image generation request timed out",
        "Add --timeout 600 and retry. Medium quality typically takes 120-300s.",
        true,
    ),
    // ── image_edit ─────────────────────────────────────────────────────
    (
        "image_edit|azure_image_transport_failed",
        "Image edit transport error",
        "Retry with --quality 'medium' or 'low'.",
        true,
    ),
    (
        "image_edit|insufficient_credit",
        "Account balance too low",
        "Top up your account and retry.",
        false,
    ),
    // ── read_image ─────────────────────────────────────────────────────
    (
        "read_image|input file too large",
        "Image file too large",
        "Resize or compress the image and retry.",
        false,
    ),
    // ── parse_doc ──────────────────────────────────────────────────────
    (
        "parse_doc|mineru_request_failed",
        "PDF parsing service is temporarily unavailable",
        "Wait a moment and retry. Alternatively, use read_image to screenshot and OCR the content.",
        true,
    ),
    (
        "parse_doc|unsupported file type",
        "Unsupported file format",
        "Only PDF (.pdf) and Word (.docx) documents are supported.",
        false,
    ),
    // ── search_paper ───────────────────────────────────────────────────
    (
        "search_paper|This operation was aborted",
        "Paper search request timed out",
        "Reduce --max_results_per_query or narrow the search scope.",
        true,
    ),
    // ── fetch_url ──────────────────────────────────────────────────────
    (
        "fetch_url|This operation was aborted",
        "Web page fetch timed out",
        "Add --timeout 120 and retry. Alternatively, try the browser tool to open the page.",
        true,
    ),
    // ── fallback (all tools) ───────────────────────────────────────────
    (
        "_default|unauthorized",
        "Not logged in or token expired",
        "Run 'future auth login' to sign in.",
        false,
    ),
    (
        "_default|401",
        "Not logged in or API key is invalid",
        "Run 'future auth login' or check the FUTURE_API_KEY environment variable.",
        false,
    ),
    (
        "_default|403",
        "Model access denied",
        "This model may not be available on your plan. Contact platform support.",
        false,
    ),
    (
        "_default|429",
        "Rate limited — too many requests",
        "Wait ~60 seconds and retry.",
        true,
    ),
    (
        "_default|insufficient_credit",
        "Account balance too low",
        "Top up your account and retry.",
        false,
    ),
    (
        "_default|This operation was aborted",
        "Request timed out",
        "Add --timeout 120 and retry.",
        true,
    ),
];

/// `translateError(toolName, rawMessage)` — case-insensitive contains match.
fn translate_error(tool_name: &str, raw_message: &str) -> Option<ErrorTranslation> {
    let lower = raw_message.to_lowercase();
    for (key, description, action, retryable) in ERROR_TRANSLATIONS {
        if let Some(pattern) = key.strip_prefix(&format!("{tool_name}|")) {
            if lower.contains(&pattern.to_lowercase()) {
                return Some(ErrorTranslation {
                    description,
                    action,
                    retryable: *retryable,
                });
            }
        }
    }
    for (key, description, action, retryable) in ERROR_TRANSLATIONS {
        if let Some(pattern) = key.strip_prefix("_default|") {
            if lower.contains(&pattern.to_lowercase()) {
                return Some(ErrorTranslation {
                    description,
                    action,
                    retryable: *retryable,
                });
            }
        }
    }
    None
}

// ── Auth ─────────────────────────────────────────────────────────────────────

/// `loadApiKey()` — FUTURE_API_KEY env → auth.json future.key → test key.
pub async fn load_api_key() -> Result<String, String> {
    if let Ok(env_key) = std::env::var("FUTURE_API_KEY") {
        if !env_key.is_empty() {
            return Ok(env_key);
        }
    }

    let not_logged_in = "Not logged in. Run \"future auth login\" first, or set the FUTURE_API_KEY environment variable.";
    let read_result: Result<String, String> = async {
        let raw = tokio::fs::read_to_string(auth_file())
            .await
            .map_err(|e| e.to_string())?;
        let auth: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if !is_record(&auth) {
            return Err("auth.json must be a JSON object".to_string());
        }
        let future = auth
            .get(FUTURE_AUTH_PROVIDER)
            .cloned()
            .unwrap_or(Value::Null);
        if !is_record(&future) {
            return Err(not_logged_in.to_string());
        }
        let key = future
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        if key.is_empty() {
            return Err(not_logged_in.to_string());
        }
        Ok(key)
    }
    .await;

    match read_result {
        Ok(key) => Ok(key),
        Err(msg) => {
            // TS: the catch block checks the test key first; only ENOENT
            // becomes "Not logged in". Parse errors rethrow.
            if let Ok(test_key) = std::env::var("FUTURE_API_TEST_KEY") {
                if !test_key.is_empty() {
                    return Ok(test_key);
                }
            }
            if msg.contains("No such file or directory") {
                return Err(not_logged_in.to_string());
            }
            Err(msg)
        }
    }
}

// ── Tool operations ─────────────────────────────────────────────────────────

/// `listRemoteTools(apiKey)` — MCP `tools/list` → [{name, description}].
async fn list_remote_tools(api_key: &str) -> Result<Vec<(String, String)>, String> {
    let session_id = initialize_session(api_key).await?;
    let response = mcp_post(
        &mcp_url().await,
        "tools/list",
        &Map::new(),
        api_key,
        Some(&session_id),
        Some(2),
        None,
    )
    .await?;

    if response.body.get("error").is_some() {
        let err = response.body.get("error").cloned().unwrap_or_default();
        let code = mcp_error_code(&err);
        let message = mcp_error_message(&err);
        return Err(format!("tools/list failed: code={code}, message={message}"));
    }
    let result = result_of(&response.body);
    let tools = result
        .and_then(|r| r.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for tool in tools.iter().filter(|t| is_record(t)) {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        out.push((name, description));
    }
    Ok(out)
}

/// `mcpErrorCode(err)` — `code` as string (number or string; else "unknown").
fn mcp_error_code(err: &Value) -> String {
    match err.get("code") {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => "unknown".to_string(),
    }
}

/// `mcpErrorMessage(err)` — `message` or "unknown error".
fn mcp_error_message(err: &Value) -> String {
    err.get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error")
        .to_string()
}

struct CallToolResponse {
    text: String,
    structured_content: Option<Value>,
}

/// `callRemoteTool(apiKey, name, args, timeoutMs)` — MCP `tools/call`.
async fn call_remote_tool(
    api_key: &str,
    name: &str,
    args: &Map<String, Value>,
    timeout_ms: Option<u64>,
) -> Result<CallToolResponse, String> {
    let session_id = initialize_session(api_key).await?;
    let mut params = Map::new();
    params.insert("name".to_string(), Value::String(name.to_string()));
    params.insert("arguments".to_string(), Value::Object(args.clone()));
    let response = mcp_post(
        &mcp_url().await,
        "tools/call",
        &params,
        api_key,
        Some(&session_id),
        Some(2),
        timeout_ms,
    )
    .await?;

    if response.body.get("error").is_some() {
        // Sanitize: only expose code and message — never leak upstream
        // internals (RequestId, HostId, nested data bodies, ...).
        let err = response.body.get("error").cloned().unwrap_or_default();
        let code = mcp_error_code(&err);
        let message = mcp_error_message(&err);
        return Err(format!("code={code}, message={message}"));
    }

    let result = result_of(&response.body);
    let content = result
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut texts: Vec<String> = Vec::new();
    for block in content.iter().filter(|b| is_record(b)) {
        let b = block.as_object().unwrap();
        let text = match b.get("type").and_then(Value::as_str) {
            Some("text") => b.get("text").map(js_string).unwrap_or_default(),
            Some("resource") => b
                .get("resource")
                .map(|r| serde_json::to_string_pretty(r).unwrap_or_default())
                .unwrap_or_default(),
            _ => serde_json::to_string_pretty(block).unwrap_or_default(),
        };
        texts.push(text);
    }

    let structured_content = result
        .and_then(|r| r.get("structuredContent"))
        .filter(|v| is_record(v))
        .cloned();

    Ok(CallToolResponse {
        text: texts.join("\n"),
        structured_content,
    })
}

/// `formatToolResult(toolName, result, outputPath)`.
async fn format_tool_result(
    tool_name: &str,
    result: &CallToolResponse,
    output_path: Option<&str>,
) -> String {
    let Some(sc) = &result.structured_content else {
        return result.text.clone();
    };
    match tool_name {
        "search_paper" => format_search_paper(sc),
        "get_paper" => format_get_paper(sc),
        "web_search" => format_web_search(sc),
        "fetch_url" => format_fetch_url(sc),
        "read_image" => format_read_image(sc),
        "parse_doc" => format_parse_doc(sc),
        "image_gen" | "image_edit" => format_image_result(tool_name, sc, output_path).await,
        _ => {
            if result.text.is_empty() {
                serde_json::to_string_pretty(sc).unwrap_or_default()
            } else {
                result.text.clone()
            }
        }
    }
}

// ── search_paper ────────────────────────────────────────────────────────────

fn format_search_paper(sc: &Value) -> String {
    let results = sc.get("results").and_then(Value::as_array);
    let Some(results) = results else {
        return "No papers found.".to_string();
    };
    if results.is_empty() {
        return "No papers found.".to_string();
    }

    let mut parts: Vec<String> = Vec::new();
    for qr in results {
        if !is_record(qr) {
            continue;
        }
        let query = js_string(&qr.get("query").cloned().unwrap_or_default());
        let papers = qr
            .get("papers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if papers.is_empty() {
            continue;
        }

        parts.push(format!(
            "## Search Results: \"{query}\" ({} papers)\n",
            papers.len()
        ));
        for (i, p) in papers.iter().enumerate() {
            let title = str_of(p.get("title"));
            let authors = str_of(p.get("authors"));
            let journal = str_of(p.get("journal"));
            let year = str_of(p.get("year"));
            let doi = str_of(p.get("doi"));
            let url = str_of(p.get("url"));
            let ai_summary = str_of(p.get("ai_summary"));

            let line = format!(
                "### {}. {}",
                i + 1,
                if title.is_empty() { "Untitled" } else { &title }
            );
            parts.push(line);
            if !authors.is_empty() {
                parts.push(format!("**Authors:** {authors}"));
            }
            if !journal.is_empty() || !year.is_empty() {
                let j = if year.is_empty() {
                    journal.clone()
                } else if journal.is_empty() {
                    format!("({year})")
                } else {
                    format!("{journal} ({year})")
                };
                parts.push(format!("**Journal:** {j}"));
            }
            if !doi.is_empty() {
                parts.push(format!("**DOI:** {doi}"));
            }
            if !url.is_empty() {
                parts.push(format!("**URL:** {url}"));
            }
            if !ai_summary.is_empty() {
                parts.push(format!("\n{ai_summary}"));
            }
            parts.push(String::new());
        }
    }
    let joined = parts.join("\n");
    let trimmed = joined.trim().to_string();
    if trimmed.is_empty() {
        "No papers found.".to_string()
    } else {
        trimmed
    }
}

// ── get_paper ───────────────────────────────────────────────────────────────

fn format_get_paper(sc: &Value) -> String {
    let Some(paper) = sc.get("paper") else {
        return "No paper found.".to_string();
    };
    if !is_record(paper) {
        return "No paper found.".to_string();
    }
    let title = str_of(paper.get("title"));
    let authors = str_of(paper.get("authors"));
    let journal = str_of(paper.get("journal"));
    let year = str_of(paper.get("year"));
    let doi = str_of(paper.get("doi"));
    let pubmed_id = str_of(paper.get("pubmed_id"));
    let url = str_of(paper.get("url"));
    let body_text = str_of(paper.get("body_text"));

    let mut parts: Vec<String> = Vec::new();
    parts.push(format!(
        "# {}",
        if title.is_empty() { "Untitled" } else { &title }
    ));
    if !authors.is_empty() {
        parts.push(format!("**Authors:** {authors}"));
    }
    if !journal.is_empty() || !year.is_empty() {
        let j = if year.is_empty() {
            journal.clone()
        } else if journal.is_empty() {
            format!("({year})")
        } else {
            format!("{journal} ({year})")
        };
        parts.push(format!("**Journal:** {j}"));
    }
    if !doi.is_empty() {
        let mut line = format!("**DOI:** {doi}");
        if !pubmed_id.is_empty() {
            line.push_str(&format!(" | **PMID:** {pubmed_id}"));
        }
        parts.push(line);
    }
    if !url.is_empty() {
        parts.push(format!("**URL:** {url}"));
    }
    parts.push(String::new());
    parts.push("---".to_string());
    parts.push(String::new());
    parts.push(if body_text.is_empty() {
        "(No body text available)".to_string()
    } else {
        body_text
    });

    parts.join("\n")
}

// ── web_search ──────────────────────────────────────────────────────────────

fn format_web_search(sc: &Value) -> String {
    let query = str_of(sc.get("query"));
    let results = sc.get("results").and_then(Value::as_array);
    let Some(results) = results else {
        return format!("## Search Results: \"{query}\"\n\nNo results found.");
    };
    if results.is_empty() {
        return format!("## Search Results: \"{query}\"\n\nNo results found.");
    }

    let mut parts: Vec<String> = Vec::new();
    parts.push(format!(
        "## Search Results: \"{query}\" ({} results)\n",
        results.len()
    ));
    for (i, r) in results.iter().enumerate() {
        if !is_record(r) {
            continue;
        }
        let title = str_of(r.get("title"));
        let link = str_of(r.get("link"));
        let snippet = str_of(r.get("snippet"));

        parts.push(format!(
            "{}. **{}**",
            i + 1,
            if title.is_empty() { "Untitled" } else { &title }
        ));
        if !link.is_empty() {
            parts.push(format!("   {link}"));
        }
        if !snippet.is_empty() {
            parts.push(format!("   {snippet}"));
        }
        parts.push(String::new());
    }
    parts.join("\n").trim().to_string()
}

// ── fetch_url ───────────────────────────────────────────────────────────────

fn format_fetch_url(sc: &Value) -> String {
    let url = str_of(sc.get("url"));
    let title = str_of(sc.get("title"));
    let content = str_of(sc.get("content"));

    let mut parts: Vec<String> = Vec::new();
    if !title.is_empty() {
        parts.push(format!("# {title}"));
    }
    parts.push(format!(
        "**URL:** {}",
        if url.is_empty() { "(unknown)" } else { &url }
    ));
    parts.push(String::new());
    parts.push(if content.is_empty() {
        "(No content)".to_string()
    } else {
        content
    });
    parts.join("\n")
}

// ── read_image / parse_doc ──────────────────────────────────────────────────

fn format_read_image(sc: &Value) -> String {
    let answer = str_of(sc.get("answer"));
    if answer.is_empty() {
        "(No answer)".to_string()
    } else {
        answer
    }
}

fn format_parse_doc(sc: &Value) -> String {
    let markdown = str_of(sc.get("markdown"));
    if markdown.is_empty() {
        "(No content)".to_string()
    } else {
        markdown
    }
}

// ── image_gen / image_edit ──────────────────────────────────────────────────

/// `IMAGE_OUTPUT_DIR` — `~/.future/agent/images`.
fn image_output_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".future")
        .join("agent")
        .join("images")
}

/// `formatImageResult(toolName, sc, outputPath)`.
async fn format_image_result(tool_name: &str, sc: &Value, output_path: Option<&str>) -> String {
    let images = sc.get("images");
    let prompt = str_of(sc.get("prompt"));
    let size = {
        let s = str_of(sc.get("size"));
        if s.is_empty() {
            "unknown".to_string()
        } else {
            s
        }
    };
    let quality = {
        let s = str_of(sc.get("quality"));
        if s.is_empty() {
            "unknown".to_string()
        } else {
            s
        }
    };
    let fmt = {
        let s = str_of(sc.get("format"));
        if s.is_empty() {
            "png".to_string()
        } else {
            s
        }
    };

    let verb = if tool_name == "image_edit" {
        "Image edited"
    } else {
        "Image generated"
    };
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("[{verb}: {size} {quality} {fmt}]"));
    if !prompt.is_empty() {
        parts.push(format!("Prompt: {prompt}"));
    }

    let mut image_list: Vec<(String, String)> = Vec::new();
    if let Some(images) = images.and_then(Value::as_array) {
        for img in images {
            if !is_record(img) {
                continue;
            }
            let b64 = img.get("b64_json").and_then(Value::as_str);
            let img_fmt = {
                let s = str_of(img.get("format"));
                if s.is_empty() {
                    fmt.clone()
                } else {
                    s
                }
            };
            if let Some(b64) = b64 {
                image_list.push((b64.to_string(), img_fmt));
            }
        }
    }
    if image_list.is_empty() {
        return parts.join("\n");
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut paths: Vec<String> = Vec::new();
    for (i, (b64, img_fmt)) in image_list.iter().enumerate() {
        let ext = if img_fmt == "jpeg" {
            "jpg"
        } else {
            img_fmt.as_str()
        };
        let file_path: PathBuf = match output_path {
            Some(output_path) => {
                if image_list.len() == 1 {
                    Path::new(output_path).to_path_buf()
                } else {
                    let dot = output_path.rfind('.');
                    match dot {
                        Some(dot) if dot > 0 => {
                            let base = &output_path[..dot];
                            let suffix = &output_path[dot..];
                            Path::new(&format!("{base}_{}{suffix}", i + 1)).to_path_buf()
                        }
                        _ => Path::new(&format!("{output_path}_{}.{ext}", i + 1)).to_path_buf(),
                    }
                }
            }
            None => {
                let suffix = if image_list.len() > 1 {
                    format!("_{}", i + 1)
                } else {
                    String::new()
                };
                image_output_dir().join(format!("future-image-{now_ms}{suffix}.{ext}"))
            }
        };

        // `fsMkdirForPath` — recursive mkdir, errors ignored. The computed
        // file_path always has a parent (a file name is always appended).
        let _ =
            tokio::fs::create_dir_all(file_path.parent().expect("file path has a parent")).await;
        // `writeFile(path, Buffer.from(b64, "base64"))`
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
            let _ = tokio::fs::write(&file_path, bytes).await;
        }
        paths.push(file_path.display().to_string());
    }

    parts.push(String::new());
    for (i, path) in paths.iter().enumerate() {
        if image_list.len() > 1 {
            parts.push(format!("Image {}: {path}", i + 1));
        } else {
            parts.push(format!("Saved: {path}"));
        }
    }

    parts.join("\n")
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// `str(v)` — strings only (empty otherwise).
fn str_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// `String(v)` — JS String() coercion for the values tools.ts stringifies
/// directly (query, text blocks): string → itself, number → shortest repr,
/// bool → true/false, everything else → "".
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

// ── parseToolArgs (port of tools.ts + tools.test.ts) ────────────────────────

/// `parseToolArgs(raw)` — parse `--args` JSON with Windows cmd.exe /
/// PowerShell quote-stripping recovery.
pub fn parse_tool_args(raw: &str) -> Result<Map<String, Value>, String> {
    let candidates = tool_arg_candidates(raw);
    let mut last_error: Option<String> = None;

    for candidate in &candidates {
        match serde_json::from_str::<Value>(candidate) {
            Ok(value) => {
                // Windows process creation can preserve an extra encoded JSON
                // layer — parse again if the value is a string.
                let value = match value {
                    Value::String(inner) => {
                        serde_json::from_str::<Value>(&inner).unwrap_or(Value::String(inner))
                    }
                    other => other,
                };
                if let Value::Object(map) = value {
                    return Ok(map);
                }
            }
            Err(e) => last_error = Some(e.to_string()),
        }
    }

    // cmd.exe can consume every double quote before the CLI receives argv,
    // leaving `{prompt:puppy,size:1024x1024}`. Recover this common flat form.
    let relaxed = parse_cmd_object(&strip_outer_quotes(raw));
    if let Some(map) = relaxed {
        return Ok(map);
    }

    Err(format!(
        "--args must be a JSON object, e.g. '{{\"prompt\":\"...\"}}' ({})",
        last_error.unwrap_or_else(|| "invalid JSON".to_string())
    ))
}

/// `toolArgCandidates(raw)` — conservative variants for cmd.exe / PowerShell
/// quote mangling, deduplicated preserving order.
fn tool_arg_candidates(raw: &str) -> Vec<String> {
    let stripped = strip_outer_quotes(raw);
    let unescaped = stripped.replace("\\\"", "\"").replace("\\'", "'");
    let mut out: Vec<String> = Vec::new();
    for candidate in [raw.trim().to_string(), stripped, unescaped] {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// `parseCmdObject(raw)` — flat key/value object from quote-stripped JSON.
fn parse_cmd_object(raw: &str) -> Option<Map<String, Value>> {
    let text = raw.trim();
    if !text.starts_with('{') || !text.ends_with('}') {
        return None;
    }

    let mut result = Map::new();
    let body = text[1..text.len() - 1].trim();
    if body.is_empty() {
        return Some(result);
    }

    for field in split_top_level(body, ',') {
        let colon = field.find(':')?;
        if colon == 0 {
            return None;
        }
        let key = field[..colon]
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        if key.is_empty() {
            return None;
        }
        let raw_value = field[colon + 1..].trim();
        result.insert(key.to_string(), parse_cmd_value(raw_value));
    }
    Some(result)
}

/// `splitTopLevel(text, sep)` — split only at brace/bracket depth 0.
fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            c if c == separator && depth == 0 => {
                parts.push(text[start..i].to_string());
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(text[start..].to_string());
    parts
}

/// `parseCmdValue(raw)` — primitive detection first, then nested parse.
fn parse_cmd_value(raw: &str) -> Value {
    let text = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    if text == "true" {
        return Value::Bool(true);
    }
    if text == "false" {
        return Value::Bool(false);
    }
    if text == "null" {
        return Value::Null;
    }
    // `/^-?\d+(?:\.\d+)?$/` → Number(text). Integers stay integers so JSON
    // serialization matches JS (5, not 5.0).
    if is_integer_literal(text) {
        if let Ok(n) = text.parse::<i64>() {
            return Value::from(n);
        }
        // (i64 overflow falls through to the float parse below.)
    }
    if is_number_literal(text) {
        // A string matching the number regex always parses as f64.
        return Value::from(text.parse::<f64>().expect("number literal parses"));
    }

    if text.starts_with('{') && text.ends_with('}') {
        if let Some(nested) = parse_cmd_object(text) {
            return Value::Object(nested);
        }
    }
    if text.starts_with('[') && text.ends_with(']') {
        let inner = text[1..text.len() - 1].trim();
        if inner.is_empty() {
            return Value::Array(Vec::new());
        }
        let items = split_top_level(inner, ',');
        return Value::Array(
            items
                .iter()
                .map(|item| parse_cmd_value(item.trim()))
                .collect(),
        );
    }

    Value::String(text.to_string())
}

fn is_integer_literal(text: &str) -> bool {
    let mut chars = text.chars();
    let rest = match chars.next() {
        Some('-') => chars.as_str(),
        Some(_) => text,
        None => return false,
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn is_number_literal(text: &str) -> bool {
    let mut seen_digit = false;
    let mut seen_dot = false;
    for (i, c) in text.chars().enumerate() {
        match c {
            '-' if i == 0 => {}
            '.' if !seen_dot => seen_dot = true,
            '0'..='9' => seen_digit = true,
            _ => return false,
        }
    }
    seen_digit
}

/// `stripOuterQuotes(input)`.
fn strip_outer_quotes(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.len() >= 2 {
        let first = trimmed.chars().next().unwrap();
        let last = trimmed.chars().last().unwrap();
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
    }
    trimmed.to_string()
}

// ── Public command entry ────────────────────────────────────────────────────

/// `isToolsCommand(command)` — type-guard port; `undefined` is not a command.
pub fn is_tools_command(command: Option<&str>) -> bool {
    matches!(command, Some("list" | "call" | "describe"))
}

/// `tools(command, args)` — port of the tools.ts command body.
pub async fn tools(command: &str, args: &[String], out: &Output) -> Result<(), String> {
    if command == "list" {
        return tools_list(args, out).await;
    }
    if command == "describe" {
        return tools_describe(args, out).await;
    }
    tools_call(args, out).await
}

// ── list ────────────────────────────────────────────────────────────────────

async fn tools_list(args: &[String], out: &Output) -> Result<(), String> {
    let json_flag = args.iter().any(|a| a == "--json");
    let mut all_tools: Vec<Value> = Vec::new();

    // Local catalog (browser)
    for (name, entry) in browser_tool_catalog() {
        all_tools.push(json!({ "name": name, "description": entry.description }));
    }

    // Remote tools from API, prefer local catalog descriptions
    let remote_result: Result<Vec<(String, String)>, String> = async {
        let api_key = load_api_key().await?;
        list_remote_tools(&api_key).await
    }
    .await;
    match remote_result {
        Ok(remote) => {
            for (name, description) in remote {
                let local = find_tool_entry(&name);
                let mut item = Map::new();
                item.insert("name".to_string(), Value::String(name));
                item.insert(
                    "description".to_string(),
                    Value::String(
                        local
                            .map(|e| e.description.to_string())
                            .unwrap_or(description),
                    ),
                );
                if let Some(entry) = local {
                    if entry.input_required {
                        item.insert("needsInput".to_string(), Value::Bool(true));
                    }
                }
                all_tools.push(Value::Object(item));
            }
        }
        Err(error) => {
            if !json_flag {
                out.log_err(&format!("Remote tools unavailable: {error}"));
                out.log_err("Showing local tools only.\n");
            }
        }
    }

    if json_flag {
        out.log(&serde_json::to_string_pretty(&Value::Array(all_tools)).unwrap_or_default());
    } else {
        let max_name = all_tools
            .iter()
            .map(|t| {
                t.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(0)
            .max(12);
        for t in &all_tools {
            let name = t.get("name").and_then(Value::as_str).unwrap_or("");
            let desc = t.get("description").and_then(Value::as_str).unwrap_or("");
            let desc_out: String = if desc.chars().count() > 90 {
                let mut s: String = desc.chars().take(89).collect();
                s.push('…');
                s
            } else {
                desc.to_string()
            };
            let hint = if t.get("needsInput").is_some() {
                " [needs --input]"
            } else {
                ""
            };
            out.log(&format!(
                "  {:<width$} {desc_out}{hint}",
                name,
                width = max_name + 2
            ));
        }
        out.log(&format!(
            "\n{} tools available.  Use \"future tools describe <name>\" for details.",
            all_tools.len()
        ));
    }
    Ok(())
}

// ── describe ────────────────────────────────────────────────────────────────

async fn tools_describe(args: &[String], out: &Output) -> Result<(), String> {
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        out.log("Usage: future tools describe <tool_name>\n\nShow arguments, flags, and usage example for a tool.");
        return Ok(());
    }
    let Some(tool_name) = args.first() else {
        out.log_err("Usage: future tools describe <tool_name>");
        out.set_exit_code(1);
        return Ok(());
    };
    let tool_name = tool_name.as_str();

    let Some(entry) = find_tool_entry(tool_name) else {
        // Fallback: try remote tool
        let found: Option<(String, String)> = async {
            match load_api_key().await {
                Ok(api_key) => match list_remote_tools(&api_key).await {
                    Ok(remote) => remote.into_iter().find(|(n, _)| n == tool_name),
                    Err(_) => None,
                },
                Err(_) => None,
            }
        }
        .await;
        if let Some((name, description)) = found {
            out.log(&format!("  {name}"));
            out.log(&format!("  {description}"));
            out.log("");
            out.log("  Remote tool — use --args with JSON to call it:");
            out.log(&format!(
                "  future tools call {name} --args '{{\"param\": \"value\"}}'"
            ));
            return Ok(());
        }
        out.log_err(&format!("Tool not found: {tool_name}"));
        return Err(crate::HANDLED_EXIT.to_string());
    };

    out.log(&format!("  {tool_name}"));
    out.log(&format!("  {}", entry.description));

    // Flags (common to all tools)
    out.log("");
    out.log("  Flags:");
    if entry.input_required {
        out.log("    --input <path>     Input file");
        if entry.mask_supported {
            out.log("    --mask <path>      Optional mask image");
        }
    }
    if entry.output_supported {
        out.log("    --output <path>    Save output to file");
    }
    out.log("    --timeout <secs>   HTTP timeout (default: 60s)");

    // Arguments (tool-specific). Every catalog tool currently declares
    // args, so the header is unconditional (the TS guarded an empty list).
    out.log("");
    out.log("  Arguments (--key value):");
    for (name, desc) in &entry.args {
        out.log(&format!("    --{name:<24} {desc}"));
    }

    // Example
    let example_flags = example_flags(entry.example);
    let input_part = if entry.input_required {
        "--input <file> "
    } else {
        ""
    };
    out.log("");
    out.log("  Example:");
    out.log(&format!(
        "  future tools call {tool_name} {input_part}{example_flags}"
    ));
    Ok(())
}

/// `exampleFlags(example)` — build `--key value` flags from the example JSON.
fn example_flags(example: &str) -> String {
    let Ok(ex) = serde_json::from_str::<Value>(example) else {
        return String::new();
    };
    let Some(obj) = ex.as_object() else {
        return String::new();
    };
    let mut flags: Vec<String> = Vec::new();
    for (k, v) in obj {
        match v {
            Value::Array(_) => flags.push(format!(
                "--{k} '{}'",
                serde_json::to_string(v).unwrap_or_default()
            )),
            Value::String(s) => flags.push(format!("--{k} \"{s}\"")),
            _ => flags.push(format!(
                "--{k} {}",
                serde_json::to_string(v).unwrap_or_default()
            )),
        }
    }
    flags.join(" ")
}

// ── call ────────────────────────────────────────────────────────────────────

async fn tools_call(args: &[String], out: &Output) -> Result<(), String> {
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        out.log("Usage: future tools call <tool_name> [--key value...]\n\nCall a tool by name. Use \"future tools describe <tool_name>\" to see\nrequired arguments, flags, and examples for each tool.");
        return Ok(());
    }
    let Some(tool_name) = args.first() else {
        out.log_err("Usage: future tools call <tool_name> [--key value...] [--input <path>] [--output <path>] [--timeout <secs>] [--raw]");
        out.set_exit_code(1);
        return Ok(());
    };
    if tool_name.starts_with("--") {
        out.log_err("Usage: future tools call <tool_name> [--key value...] [--input <path>] [--output <path>] [--timeout <secs>] [--raw]");
        out.set_exit_code(1);
        return Ok(());
    }
    let tool_name = tool_name.as_str();

    let mut tool_args: Map<String, Value> = Map::new();
    let stdin_flag = args.iter().any(|a| a == "--stdin");
    let output_idx = args.iter().position(|a| a == "--output");
    let output_path = output_idx.and_then(|i| args.get(i + 1)).cloned();
    let input_idx = args.iter().position(|a| a == "--input");
    let input_path = input_idx.and_then(|i| {
        let next = args.get(i + 1)?;
        if next.starts_with("--") {
            None
        } else {
            Some(next.clone())
        }
    });
    let mask_idx = args.iter().position(|a| a == "--mask");
    let mask_path = mask_idx.and_then(|i| {
        let next = args.get(i + 1)?;
        if next.starts_with("--") {
            None
        } else {
            Some(next.clone())
        }
    });
    let timeout_idx = args.iter().position(|a| a == "--timeout");
    let timeout_sec: i64 = timeout_idx
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let timeout_ms: Option<u64> = if timeout_sec > 0 {
        Some((timeout_sec * 1000) as u64)
    } else if matches!(tool_name, "image_gen" | "image_edit") {
        Some(600_000)
    } else {
        None
    };

    let raw_flag = args.iter().any(|a| a == "--raw");

    if stdin_flag {
        let mut stdin_bytes = Vec::new();
        let _ = tokio::io::stdin().read_to_end(&mut stdin_bytes).await;
        tool_args = parse_tool_args(&String::from_utf8_lossy(&stdin_bytes))?;
    }

    // Tool arguments: --key value.
    const KNOWN_FLAGS: &[&str] = &[
        "--stdin",
        "--input",
        "--mask",
        "--output",
        "--timeout",
        "--raw",
    ];
    let mut i = 1usize;
    while i + 1 < args.len() {
        let arg = args[i].as_str();
        if arg.starts_with("--") && !KNOWN_FLAGS.contains(&arg) {
            let val = args[i + 1].as_str();
            if !val.starts_with("--") {
                tool_args.insert(arg[2..].to_string(), parse_cmd_value(val));
                i += 1;
            }
        }
        i += 1;
    }

    // Resolve --input / --mask flags to base64, tool-aware:
    //   image_edit, read_image → image_b64 / mask_b64
    //   parse_doc              → doc_b64
    if let Some(input_path) = &input_path {
        match tokio::fs::read(input_path).await {
            Ok(buf) => {
                let b64_key = if tool_name == "parse_doc" {
                    "doc_b64"
                } else {
                    "image_b64"
                };
                tool_args.insert(
                    b64_key.to_string(),
                    Value::String(base64::engine::general_purpose::STANDARD.encode(buf)),
                );
            }
            Err(_) => {
                out.log_err(&format!("Error: cannot read input file: {input_path}"));
                return Err(crate::HANDLED_EXIT.to_string());
            }
        }
    }
    if let Some(mask_path) = &mask_path {
        match tokio::fs::read(mask_path).await {
            Ok(buf) => {
                tool_args.insert(
                    "mask_b64".to_string(),
                    Value::String(base64::engine::general_purpose::STANDARD.encode(buf)),
                );
            }
            Err(_) => {
                out.log_err(&format!("Error: cannot read mask file: {mask_path}"));
                return Err(crate::HANDLED_EXIT.to_string());
            }
        }
    }

    // Pre-check: for known tools, validate required args and value ranges
    if let Some(catalog_entry) = find_tool_entry(tool_name) {
        let missing: Vec<&str> = catalog_entry
            .args
            .iter()
            .filter(|(name, desc)| desc.contains("required") && !tool_args.contains_key(*name))
            .map(|(name, _)| *name)
            .collect();
        if !missing.is_empty() {
            out.log_err(&format!(
                "Error: {tool_name} requires: {}",
                missing
                    .iter()
                    .map(|m| format!("--{m}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            out.log_err(&format!(
                "Use \"future tools describe {tool_name}\" for details."
            ));
            return Err(crate::HANDLED_EXIT.to_string());
        }

        // Validate numeric ranges for known parameters
        let int_range = |key: &str, min: i64, max: i64, out: &Output| -> Result<(), String> {
            let Some(v) = tool_args.get(key) else {
                return Ok(());
            };
            if matches!(v, Value::Number(n) if n.as_i64().is_some_and(|n| n >= min && n <= max)) {
                return Ok(());
            }
            out.log_err(&format!(
                "Error: --{key} must be an integer between {min} and {max}, got: {}",
                serde_json::to_string(v).unwrap_or_default()
            ));
            Err(crate::HANDLED_EXIT.to_string())
        };
        let int_min = |key: &str, min: i64, out: &Output| -> Result<(), String> {
            let Some(v) = tool_args.get(key) else {
                return Ok(());
            };
            if matches!(v, Value::Number(n) if n.as_i64().is_some_and(|n| n >= min)) {
                return Ok(());
            }
            out.log_err(&format!(
                "Error: --{key} must be a positive integer, got: {}",
                serde_json::to_string(v).unwrap_or_default()
            ));
            Err(crate::HANDLED_EXIT.to_string())
        };

        // Validate search_paper queries
        if tool_name == "search_paper" && tool_args.contains_key("queries") {
            let q = tool_args.get("queries");
            let ok = matches!(q, Some(Value::Array(items)) if !items.is_empty()
                && items.iter().all(|s| s.as_str().is_some_and(|s| !s.trim().is_empty())));
            if !ok {
                out.log_err("Error: --queries must be a non-empty array of non-empty strings");
                return Err(crate::HANDLED_EXIT.to_string());
            }
        }

        int_range("n", 1, 10, out)?;
        int_range("count", 1, 50, out)?;
        int_min("max_k", 1, out)?;
        int_min("max_tokens", 1, out)?;

        // Normalize file_type to lowercase
        if let Some(Value::String(ft)) = tool_args.get("file_type") {
            let ft_lower = ft.to_lowercase();
            if ft_lower != "pdf" && ft_lower != "docx" {
                out.log_err(&format!(
                    "Error: --file_type must be \"pdf\" or \"docx\", got: \"{ft}\""
                ));
                return Err(crate::HANDLED_EXIT.to_string());
            }
            tool_args.insert("file_type".to_string(), Value::String(ft_lower));
        }
    }

    // Validate --timeout (common flag)
    if timeout_sec < 0 {
        out.log_err(&format!(
            "Error: --timeout must be >= 1 second, got: {timeout_sec}"
        ));
        return Err(crate::HANDLED_EXIT.to_string());
    }

    if is_browser_tool(tool_name) {
        let (output, exit_code) = match call_browser_tool(tool_name, &tool_args, out).await {
            Ok(result) => {
                // Every browser tool result carries structured content (the
                // text fallback in the TS is unreachable in the Rust port).
                let output =
                    serde_json::to_string_pretty(&result.structured_content).unwrap_or_default();
                (output, 0)
            }
            Err(message) => (message, 1),
        };
        // `writeSync(exitCode === 0 ? 1 : 2, \`${output}\n\`)` then
        // `process.exit(exitCode)` — no generic error printing.
        if exit_code == 0 {
            out.log(&output);
            return Ok(());
        }
        out.log_err(&output);
        return Err(crate::HANDLED_EXIT.to_string());
    }

    let api_key = load_api_key().await?;
    let result = match call_remote_tool(&api_key, tool_name, &tool_args, timeout_ms).await {
        Ok(result) => result,
        Err(raw_msg) => {
            let translation = translate_error(tool_name, &raw_msg);
            match translation {
                Some(t) => {
                    out.log_err(&format!("Error: {}", t.description));
                    out.log_err(&format!("Fix: {}", t.action));
                    if t.retryable {
                        out.log_err("(This is usually temporary — retry should work.)");
                    }
                }
                None => {
                    out.log_err(&format!("Error calling {tool_name}: {raw_msg}"));
                }
            }
            out.log_err(&format!(
                "Use \"future tools describe {tool_name}\" for help."
            ));
            return Err(crate::HANDLED_EXIT.to_string());
        }
    };

    // --raw: output the original MCP result directly; otherwise format it.
    if raw_flag {
        let has_sc = result
            .structured_content
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|m| !m.is_empty());
        if has_sc {
            out.log(
                &serde_json::to_string_pretty(result.structured_content.as_ref().unwrap())
                    .unwrap_or_default(),
            );
        } else {
            out.log(&result.text);
        }
    } else {
        out.log(&format_tool_result(tool_name, &result, output_path.as_deref()).await);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `deepEqual` via serde_json Value equality.
    fn parse(raw: &str) -> Value {
        Value::Object(parse_tool_args(raw).unwrap_or_else(|e| panic!("parse failed: {e}")))
    }

    #[test]
    fn simple_valid_json() {
        let result = parse(r#"{"prompt":"hello"}"#);
        assert_eq!(result, json!({"prompt": "hello"}));
    }

    #[test]
    fn multiple_keys() {
        let result = parse(r#"{"prompt":"hello","size":"1024x1024"}"#);
        assert_eq!(result, json!({"prompt": "hello", "size": "1024x1024"}));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn numeric_and_boolean_values() {
        let result = parse(r#"{"n":5,"flag":true,"ratio":3.14}"#);
        assert_eq!(result, json!({"n": 5, "flag": true, "ratio": 3.14}));
    }

    #[test]
    fn single_quote_wrapped_json() {
        let result = parse(r#"'{"prompt":"hello"}'"#);
        assert_eq!(result, json!({"prompt": "hello"}));
    }

    #[test]
    fn cmd_exe_stripped_quotes_no_spaces() {
        let result = parse("{prompt:hello}");
        assert_eq!(result, json!({"prompt": "hello"}));
    }

    #[test]
    fn cmd_exe_stripped_quotes_single_quote_wrapping() {
        let result = parse("'{prompt:hello,size:1024x1024}'");
        assert_eq!(result, json!({"prompt": "hello", "size": "1024x1024"}));
    }

    #[test]
    fn cmd_exe_stripped_quotes_value_with_spaces() {
        let result = parse("{prompt:a beautiful fox,size:1024x1024}");
        assert_eq!(
            result,
            json!({"prompt": "a beautiful fox", "size": "1024x1024"})
        );
    }

    #[test]
    fn cmd_exe_stripped_quotes_leading_trailing_single_quotes() {
        let result = parse("'{prompt:a beautiful fox}'");
        assert_eq!(result, json!({"prompt": "a beautiful fox"}));
    }

    #[test]
    fn valid_json_with_comma_in_value() {
        let result = parse(r#"{"prompt":"hello, world"}"#);
        assert_eq!(result, json!({"prompt": "hello, world"}));
    }

    #[test]
    fn valid_json_with_commas_in_multiple_values() {
        let result = parse(r#"{"prompt":"hello, world","style":"calm, serene"}"#);
        assert_eq!(
            result,
            json!({"prompt": "hello, world", "style": "calm, serene"})
        );
    }

    #[test]
    fn nested_object_value() {
        let result = parse("{messages:[{role:user,content:hello}]}");
        let messages = result.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages[0], json!({"role": "user", "content": "hello"}));
    }

    #[test]
    fn multiple_nested_fields_with_comma() {
        let result = parse("{messages:[{role:user,content:hello world}],model:gpt-4}");
        assert_eq!(result.get("model").unwrap(), &json!("gpt-4"));
        let messages = result.get("messages").unwrap().as_array().unwrap();
        assert_eq!(
            messages[0],
            json!({"role": "user", "content": "hello world"})
        );
    }

    #[test]
    fn powershell_backslash_escaped_json() {
        let result = parse(r#"{\"prompt\":\"hello\"}"#);
        assert_eq!(result, json!({"prompt": "hello"}));
    }

    #[test]
    fn empty_object() {
        let result = parse("{}");
        assert_eq!(result, json!({}));
    }

    #[test]
    fn null_value() {
        let result = parse("{key:null}");
        assert_eq!(result.get("key"), Some(&Value::Null));
    }

    #[test]
    fn boolean_false_value() {
        let result = parse("{flag:false}");
        assert_eq!(result.get("flag"), Some(&Value::Bool(false)));
    }

    #[test]
    fn error_includes_helpful_message() {
        let err = parse_tool_args("not json at all").unwrap_err();
        assert!(
            err.contains("--args must be a JSON object"),
            "message: {err}"
        );
    }

    /// Simulate the tools() function joining split argv elements back
    /// together after cmd.exe split them on spaces.
    fn simulate_rejoined_args(argv: &[&str]) -> Value {
        let args_idx = argv
            .iter()
            .position(|a| *a == "--args")
            .expect("--args not found");
        let mut raw = argv[args_idx + 1].to_string();
        let mut i = args_idx + 2;
        while i < argv.len() && !argv[i].starts_with("--") {
            raw.push(' ');
            raw.push_str(argv[i]);
            i += 1;
        }
        Value::Object(parse_tool_args(&raw).unwrap())
    }

    #[test]
    fn e2e_cmd_exe_split_on_prompt_spaces() {
        let result =
            simulate_rejoined_args(&["--args", "'{prompt:a", "beautiful", "fox,size:1024x1024}'"]);
        assert_eq!(
            result,
            json!({"prompt": "a beautiful fox", "size": "1024x1024"})
        );
    }

    #[test]
    fn e2e_cmd_exe_split_on_multiple_space_values() {
        let result = simulate_rejoined_args(&[
            "--args",
            "'{prompt:ancient",
            "oak",
            "tree,style:oil",
            "painting}'",
        ]);
        assert_eq!(
            result,
            json!({"prompt": "ancient oak tree", "style": "oil painting"})
        );
    }

    #[test]
    fn e2e_flag_after_args_is_not_joined() {
        let result =
            simulate_rejoined_args(&["--args", "'{prompt:hello}'", "--output", "result.png"]);
        assert_eq!(result, json!({"prompt": "hello"}));
    }

    #[test]
    fn example_flags_builder() {
        assert_eq!(
            example_flags(r#"{"command": "open", "url": "https://example.com"}"#),
            "--command \"open\" --url \"https://example.com\""
        );
        assert_eq!(
            example_flags(r#"{"queries": ["a", "b"], "n": 8}"#),
            "--queries '[\"a\",\"b\"]' --n 8"
        );
        assert_eq!(example_flags("not json"), "");
    }

    #[test]
    fn error_translation_case_insensitive() {
        let t = translate_error("image_gen", "Error: azure_image_transport_failed");
        assert!(t.is_some());
        assert_eq!(
            t.as_ref().unwrap().description,
            "Image generation transport error (remote renderer failure)"
        );
        assert!(t.as_ref().unwrap().retryable);

        // _default fallback matches substrings
        let t = translate_error("web_search", "code=401, message=unauthorized");
        assert!(t.is_some());
        assert_eq!(
            t.as_ref().unwrap().description,
            "Not logged in or token expired"
        );

        // tool-specific wins over _default
        let t = translate_error("image_gen", "insufficient_credit");
        assert_eq!(
            t.as_ref().unwrap().action,
            "Top up your account and retry. Run 'future account balance' to check."
        );

        assert!(translate_error("web_search", "no match here").is_none());
    }

    #[tokio::test]
    async fn format_image_result_single_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("out.png");
        // base64 for the 1x1 red PNG
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let sc = json!({
            "images": [{"b64_json": b64, "format": "png"}],
            "prompt": "a red fox",
            "size": "1024x1024",
            "quality": "medium",
        });
        let result = format_image_result("image_gen", &sc, Some(out_path.to_str().unwrap())).await;
        assert!(
            result.starts_with(
                "[Image generated: 1024x1024 medium png]\nPrompt: a red fox\n\nSaved: "
            ),
            "got: {result}"
        );
        assert!(tokio::fs::metadata(&out_path).await.is_ok());
    }

    #[tokio::test]
    async fn format_image_result_multi_images_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("out.png");
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let sc = json!({
            "images": [{"b64_json": b64, "format": "png"}, {"b64_json": b64, "format": "jpeg"}],
        });
        let result = format_image_result("image_gen", &sc, Some(out_path.to_str().unwrap())).await;
        // Multi-image with an --output path: suffix before the OUTPUT path's
        // extension (TS quirk — `suffix = outputPath.slice(dot)`, the image
        // format's extension only applies when the output path has none).
        assert!(
            result.contains("Image 1: ") && result.contains("_1.png"),
            "got: {result}"
        );
        assert!(
            result.contains("Image 2: ") && result.contains("_2.png"),
            "got: {result}"
        );
        assert!(tokio::fs::metadata(dir.path().join("out_1.png"))
            .await
            .is_ok());
        assert!(tokio::fs::metadata(dir.path().join("out_2.png"))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn format_image_result_no_images_returns_header() {
        let sc = json!({"prompt": "x"});
        let result = format_image_result("image_edit", &sc, None).await;
        assert_eq!(result, "[Image edited: unknown unknown png]\nPrompt: x");
    }

    // ── helpers ─────────────────────────────────────────────────────

    #[test]
    fn str_of_and_js_string_coercion() {
        assert_eq!(str_of(Some(&json!("s"))), "s");
        assert_eq!(str_of(Some(&json!(1))), "");
        assert_eq!(str_of(None), "");
        assert_eq!(js_string(&json!("s")), "s");
        assert_eq!(js_string(&json!(42)), "42");
        assert_eq!(js_string(&json!(true)), "true");
        assert_eq!(js_string(&json!(null)), "");
        assert_eq!(js_string(&json!([1])), "");
    }

    #[test]
    fn mcp_error_code_and_message() {
        assert_eq!(mcp_error_code(&json!({"code": -32600})), "-32600");
        assert_eq!(mcp_error_code(&json!({"code": "E"})), "E");
        assert_eq!(mcp_error_code(&json!({})), "unknown");
        assert_eq!(mcp_error_message(&json!({"message": "m"})), "m");
        assert_eq!(mcp_error_message(&json!({})), "unknown error");
    }

    #[test]
    fn find_tool_entry_and_image_output_dir() {
        assert!(find_tool_entry("search_paper").is_some());
        assert!(find_tool_entry("no-such-tool").is_none());
        // Browser tools are merged into the catalog too.
        assert!(find_tool_entry("browser").is_some());
        assert!(!browser_tool_catalog().is_empty());
        let dir = image_output_dir();
        assert!(dir.ends_with(".future/agent/images") || dir.ends_with(".future\\agent\\images"));
    }

    #[test]
    fn translate_error_tool_specific_and_default() {
        // Tool-specific pattern wins.
        let t = translate_error("image_gen", "azure_image_transport_failed: boom").unwrap();
        assert!(t.retryable);
        assert!(t.description.contains("transport"));
        // Default fallback for unknown tools.
        let t = translate_error("whatever", "HTTP 429 too many").unwrap();
        assert!(t.description.contains("Rate limited"));
        // Case-insensitive.
        let t = translate_error("parse_doc", "UNSUPPORTED FILE TYPE").unwrap();
        assert!(!t.retryable);
        // No match → None.
        assert!(translate_error("search_paper", "something else entirely").is_none());
    }

    // ── format_* structured-content renderers ───────────────────────

    #[test]
    fn format_search_paper_variants() {
        // Missing / empty results.
        assert_eq!(format_search_paper(&json!({})), "No papers found.");
        assert_eq!(
            format_search_paper(&json!({"results": []})),
            "No papers found."
        );
        // Results rows with no papers are skipped; all-empty → fallback.
        assert_eq!(
            format_search_paper(&json!({"results": [{"query": "q", "papers": []}]})),
            "No papers found."
        );
        // Non-object rows are skipped too.
        assert_eq!(
            format_search_paper(&json!({"results": ["junk", 42, null]})),
            "No papers found."
        );
        let sc = json!({"results": [{
            "query": "crispr",
            "papers": [
                {"title": "T", "authors": "A B", "journal": "Nat", "year": "2025", "doi": "10.1/x", "url": "http://u", "ai_summary": "sum"},
                {"title": "", "journal": "J"},
                {"year": "2024"},
                {"authors": "Solo"}
            ]
        }]});
        let out = format_search_paper(&sc);
        assert!(
            out.contains("## Search Results: \"crispr\" (4 papers)"),
            "out: {out}"
        );
        assert!(out.contains("### 1. T"), "out: {out}");
        assert!(out.contains("**Authors:** A B"));
        assert!(out.contains("**Journal:** Nat (2025)"));
        assert!(out.contains("**DOI:** 10.1/x"));
        assert!(out.contains("**URL:** http://u"));
        assert!(out.contains("\nsum"));
        assert!(out.contains("### 2. Untitled"), "out: {out}");
        assert!(
            out.contains("**Journal:** J\n") || out.contains("**Journal:** J"),
            "out: {out}"
        );
        assert!(out.contains("**Journal:** (2024)"), "out: {out}");
    }

    #[test]
    fn format_get_paper_variants() {
        assert_eq!(format_get_paper(&json!({})), "No paper found.");
        assert_eq!(format_get_paper(&json!({"paper": [1]})), "No paper found.");
        let sc = json!({"paper": {
            "title": "Paper T", "authors": "A", "journal": "J", "year": "2025",
            "doi": "10.1/x", "pubmed_id": "123", "url": "http://u", "body_text": "BODY"
        }});
        let out = format_get_paper(&sc);
        assert!(out.contains("# Paper T"));
        assert!(out.contains("**DOI:** 10.1/x | **PMID:** 123"));
        assert!(out.contains("BODY"));
        // Minimal: no title, no body.
        let out = format_get_paper(&json!({"paper": {}}));
        assert!(out.contains("# Untitled"), "out: {out}");
        assert!(out.contains("(No body text available)"), "out: {out}");
        // Journal without year; year without journal.
        assert!(format_get_paper(&json!({"paper": {"journal": "J"}})).contains("**Journal:** J\n"));
        assert!(
            format_get_paper(&json!({"paper": {"year": "2024"}})).contains("**Journal:** (2024)")
        );
    }

    #[test]
    fn format_web_search_variants() {
        assert_eq!(
            format_web_search(&json!({"query": "q"})),
            "## Search Results: \"q\"\n\nNo results found."
        );
        assert_eq!(
            format_web_search(&json!({"query": "q", "results": []})),
            "## Search Results: \"q\"\n\nNo results found."
        );
        let sc = json!({"query": "q", "results": [
            {"title": "T", "link": "http://l", "snippet": "s"},
            {"link": "http://only-link"},
            "not-a-record"
        ]});
        let out = format_web_search(&sc);
        assert!(
            out.contains("## Search Results: \"q\" (3 results)"),
            "out: {out}"
        );
        assert!(out.contains("1. **T**"));
        assert!(out.contains("   http://l"));
        assert!(out.contains("   s"));
        assert!(out.contains("2. **Untitled**"), "out: {out}");
    }

    #[test]
    fn format_fetch_url_variants() {
        // No title → heading omitted; empty content → placeholder.
        let out = format_fetch_url(&json!({}));
        assert!(out.contains("**URL:** (unknown)"), "out: {out}");
        assert!(out.contains("(No content)"), "out: {out}");
        assert!(!out.contains("# "), "out: {out}");
        let out = format_fetch_url(&json!({"url": "http://u", "title": "T", "content": "C"}));
        assert!(out.contains("# T"));
        assert!(out.contains("**URL:** http://u"));
        assert!(out.contains("C"));
    }

    #[test]
    fn format_read_image_and_parse_doc() {
        assert_eq!(format_read_image(&json!({})), "(No answer)");
        assert_eq!(format_read_image(&json!({"answer": "A"})), "A");
        assert_eq!(format_parse_doc(&json!({})), "(No content)");
        assert_eq!(format_parse_doc(&json!({"markdown": "# MD"})), "# MD");
    }

    #[tokio::test]
    async fn format_tool_result_routing() {
        let make = |text: &str, sc: Option<Value>| CallToolResponse {
            text: text.to_string(),
            structured_content: sc,
        };
        // No structured content → raw text.
        assert_eq!(
            format_tool_result("web_search", &make("plain", None), None).await,
            "plain"
        );
        // Known tools route to their renderer.
        assert_eq!(
            format_tool_result("web_search", &make("", Some(json!({"query": "q"}))), None).await,
            "## Search Results: \"q\"\n\nNo results found."
        );
        assert_eq!(
            format_tool_result("read_image", &make("", Some(json!({"answer": "A"}))), None).await,
            "A"
        );
        // Unknown tool: structured content pretty-printed when text is empty…
        let out = format_tool_result("mystery", &make("", Some(json!({"a": 1}))), None).await;
        assert!(out.contains("\"a\": 1"), "out: {out}");
        // …and text preferred when present.
        assert_eq!(
            format_tool_result("mystery", &make("txt", Some(json!({"a": 1}))), None).await,
            "txt"
        );
    }

    // ── load_api_key ────────────────────────────────────────────────

    #[tokio::test]
    async fn load_api_key_resolution_order() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // 1. FUTURE_API_KEY wins.
        let _env = crate::test_env::EnvGuard::set(&[
            ("FUTURE_API_KEY", std::ffi::OsString::from("env-key")),
            ("FUTURE_API_TEST_KEY", std::ffi::OsString::from("")),
        ]);
        assert_eq!(load_api_key().await.unwrap(), "env-key");
        drop(_env);
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
        // 2. auth.json key.
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "{\"future\": {\"key\": \"file-key\"}}")
            .await
            .unwrap();
        assert_eq!(load_api_key().await.unwrap(), "file-key");
        // 3. No key → not-logged-in message.
        tokio::fs::write(&path, "{\"future\": {}}").await.unwrap();
        let err = load_api_key().await.unwrap_err();
        assert!(err.contains("Not logged in"), "err: {err}");
        // future not an object.
        tokio::fs::write(&path, "{\"future\": 5}").await.unwrap();
        assert!(load_api_key().await.unwrap_err().contains("Not logged in"));
        // Non-object auth.json → error message.
        tokio::fs::write(&path, "[1]").await.unwrap();
        let err = load_api_key().await.unwrap_err();
        assert!(err.contains("must be a JSON object"), "err: {err}");
        // Invalid JSON → parse error propagates.
        tokio::fs::write(&path, "{bad").await.unwrap();
        assert!(!load_api_key().await.unwrap_err().is_empty());
        // 4. ENOENT → not-logged-in; FUTURE_API_TEST_KEY rescues.
        tokio::fs::remove_file(&path).await.unwrap();
        let err = load_api_key().await.unwrap_err();
        assert!(err.contains("Not logged in"), "err: {err}");
        let _env2 = crate::test_env::EnvGuard::set(&[(
            "FUTURE_API_TEST_KEY",
            std::ffi::OsString::from("test-key"),
        )]);
        assert_eq!(load_api_key().await.unwrap(), "test-key");
        // Empty FUTURE_API_KEY is ignored (JS falsy).
        let _env3 =
            crate::test_env::EnvGuard::set(&[("FUTURE_API_KEY", std::ffi::OsString::from(""))]);
        assert_eq!(load_api_key().await.unwrap(), "test-key");
        // An EMPTY FUTURE_API_TEST_KEY does not rescue either.
        drop(_env3);
        let _env4 = crate::test_env::EnvGuard::set(&[(
            "FUTURE_API_TEST_KEY",
            std::ffi::OsString::from(""),
        )]);
        let err = load_api_key().await.unwrap_err();
        assert!(err.contains("Not logged in"), "err: {err}");
    }

    // ── MCP-backed list/describe/call ───────────────────────────────

    /// Auth env: FUTURE_API_KEY set + platform pointed at the mock.
    async fn mcp_env(base: &str) -> (crate::test_env::EnvGuard, crate::test_env::EnvGuard) {
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            format!("{{\"future\": {{\"key\": \"sk\", \"base_url\": \"{base}\"}}}}"),
        )
        .await
        .unwrap();
        let guard =
            crate::test_env::EnvGuard::set(&[("FUTURE_API_KEY", std::ffi::OsString::from("sk"))]);
        (guard, crate::test_env::EnvGuard::set(&[]))
    }

    #[tokio::test]
    async fn tools_list_text_and_json_with_remote() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    // initialize, notifications/initialized, tools/list —
                    // twice: the text-mode and json-mode invocations each run
                    // the full MCP handshake chain.
                    ("data: {\"result\":{}}\n\n", Some("sess-1")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"tools\":[{\"name\":\"search_paper\",\"description\":\"REMOTE desc\"},{\"name\":\"image_edit\",\"description\":\"remote image_edit\"},{\"name\":\"remote_only\"},{\"name\":42}]}}\n\n", None),
                    ("data: {\"result\":{}}\n\n", Some("sess-2")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"tools\":[{\"name\":\"search_paper\",\"description\":\"REMOTE desc\"},{\"name\":\"image_edit\",\"description\":\"remote image_edit\"},{\"name\":\"remote_only\"},{\"name\":42}]}}\n\n", None),
                ],
            ),
        ])
        .await;
        let _env = mcp_env(&base).await;

        let (out, cap) = Output::memory();
        tools("list", &[], &out).await.expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // Local description wins over the remote one for known tools.
        assert!(
            stdout.contains("Search academic papers"),
            "stdout: {stdout}"
        );
        assert!(!stdout.contains("REMOTE desc"), "stdout: {stdout}");
        // Remote-only tool listed with its (default empty) description.
        assert!(stdout.contains("remote_only"), "stdout: {stdout}");
        assert!(stdout.contains("[needs --input]"), "stdout: {stdout}");
        assert!(stdout.contains("tools available."), "stdout: {stdout}");

        // JSON mode.
        let (out, cap) = Output::memory();
        tools("list", &["--json".to_string()], &out)
            .await
            .expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        let arr = parsed.as_array().unwrap();
        let remote_only = arr.iter().find(|t| t["name"] == "remote_only").unwrap();
        assert_eq!(remote_only["description"], "");
        assert!(remote_only.get("needsInput").is_none());
        let image_edit = arr.iter().find(|t| t["name"] == "image_edit").unwrap();
        assert_eq!(image_edit["needsInput"], true);
    }

    #[tokio::test]
    async fn tools_list_remote_unavailable_paths() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // No auth at all → load_api_key fails → warning in text mode…
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
        let (out, cap) = Output::memory();
        tools("list", &[], &out).await.expect("list");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Remote tools unavailable:"),
            "stderr: {stderr}"
        );
        assert!(
            stderr.contains("Showing local tools only."),
            "stderr: {stderr}"
        );
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(!stdout.contains("search_paper"), "stdout: {stdout}");

        // …and silently omitted in JSON mode (local tools only).
        let (out, cap) = Output::memory();
        tools("list", &["--json".to_string()], &out)
            .await
            .expect("list");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.is_empty(), "stderr: {stderr}");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert!(parsed
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "search_paper"));

        // Remote call fails (bad gateway) → same warning path with the error.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/api/v1/mcp",
            502,
            "{}",
        )])
        .await;
        let _env2 = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        tools("list", &[], &out).await.expect("list");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("502"), "stderr: {stderr}");

        // tools/list returns an error object → wrapped message.
        let base =
            crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    (
                        "data: {\"error\":{\"code\":-1,\"message\":\"list broke\"}}\n\n",
                        None,
                    ),
                ],
            )])
            .await;
        let _env3 = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        tools("list", &[], &out).await.expect("list");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("tools/list failed: code=-1, message=list broke"),
            "stderr: {stderr}"
        );
    }

    #[tokio::test]
    async fn tools_describe_variants() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // --help.
        let (out, cap) = Output::memory();
        tools("describe", &["--help".to_string()], &out)
            .await
            .unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Usage: future tools describe"));
        // Missing name → usage + exit code.
        let (out, cap) = Output::memory();
        tools("describe", &[], &out).await.unwrap();
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Usage: future tools describe"));

        // Local tool: flags + args + example.
        let (out, cap) = Output::memory();
        tools("describe", &["image_edit".to_string()], &out)
            .await
            .unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("  image_edit"), "stdout: {stdout}");
        assert!(stdout.contains("--input <path>"), "stdout: {stdout}");
        assert!(stdout.contains("--mask <path>"), "stdout: {stdout}");
        assert!(stdout.contains("--output <path>"), "stdout: {stdout}");
        assert!(stdout.contains("--timeout <secs>"), "stdout: {stdout}");
        assert!(stdout.contains("--prompt"), "stdout: {stdout}");
        assert!(
            stdout.contains("future tools call image_edit --input <file>"),
            "stdout: {stdout}"
        );

        // search_paper example renders array JSON + quoted string flags.
        let (out, cap) = Output::memory();
        tools("describe", &["search_paper".to_string()], &out)
            .await
            .unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("--queries '["), "stdout: {stdout}");
        assert!(
            stdout.contains("--max_results_per_query 8"),
            "stdout: {stdout}"
        );

        // Unknown tool with no remote → not found error.
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
        let (out, cap) = Output::memory();
        let err = tools("describe", &["nope".to_string()], &out)
            .await
            .unwrap_err();
        assert_eq!(err, crate::HANDLED_EXIT);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Tool not found: nope"), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn tools_describe_remote_failure_falls_back_to_none() {
        // API key present but the MCP endpoint is dead → the remote lookup
        // errors → None → "Tool not found".
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let (_e1, _e2) = mcp_env("http://127.0.0.1:1").await;
        let (out, cap) = Output::memory();
        let err = tools("describe", &["nope".to_string()], &out)
            .await
            .unwrap_err();
        assert_eq!(err, crate::HANDLED_EXIT);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Tool not found: nope"), "stderr: {stderr}");
    }

    #[test]
    fn cmd_value_parse_remaining_arms() {
        // Integer literal overflowing i64 falls through to the f64 parse.
        assert_eq!(parse_cmd_value("99999999999999999999999999"), json!(1e26));
        // Float literal.
        assert_eq!(parse_cmd_value("2.5"), json!(2.5));
        // Object-looking text with an unparseable field → falls back to string.
        assert_eq!(parse_cmd_value("{abc}"), json!("{abc}"));
        // strip_outer_quotes with a lone character is returned as-is.
        assert_eq!(strip_outer_quotes("x"), "x");
        // parse_cmd_object with an empty body → Some(empty map).
        assert_eq!(parse_cmd_object("{}"), Some(Map::new()));
    }

    #[tokio::test]
    async fn tools_call_trailing_input_flag_and_valid_ranges() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);

        // Trailing --input with no value → input_path None (no panic).
        let (out, _) = Output::memory();
        let _ = tools(
            "call",
            &["web_search".to_string(), "--input".to_string()],
            &out,
        )
        .await;

        // --input followed by another flag → treated as missing too.
        let (out, _) = Output::memory();
        let _ = tools(
            "call",
            &[
                "web_search".to_string(),
                "--input".to_string(),
                "--output".to_string(),
                "x.png".to_string(),
            ],
            &out,
        )
        .await;

        // --mask: with a value, followed by a flag, and trailing.
        for args in [
            vec!["image_gen", "--prompt", "p", "--mask", "m.png"],
            vec!["image_gen", "--prompt", "p", "--mask", "--output"],
            vec!["image_gen", "--prompt", "p", "--mask"],
        ] {
            let (out, _) = Output::memory();
            let _ = tools(
                "call",
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                &out,
            )
            .await;
        }

        // Valid numeric ranges pass validation (fails later at the API key).
        let (out, cap) = Output::memory();
        let _ = tools(
            "call",
            &[
                "image_gen".to_string(),
                "--prompt".to_string(),
                "x".to_string(),
                "--n".to_string(),
                "5".to_string(),
            ],
            &out,
        )
        .await;
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(!stderr.contains("must be an integer between"), "{stderr}");

        // int_min invalid + valid arms.
        let (out, cap) = Output::memory();
        let _ = tools(
            "call",
            &[
                "search_paper".to_string(),
                "--queries".to_string(),
                "[\"q\"]".to_string(),
                "--max_k".to_string(),
                "0".to_string(),
            ],
            &out,
        )
        .await;
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("--max_k must be a positive integer"),
            "{stderr}"
        );

        let (out, cap) = Output::memory();
        let _ = tools(
            "call",
            &[
                "search_paper".to_string(),
                "--queries".to_string(),
                "[\"q\"]".to_string(),
                "--max_k".to_string(),
                "5".to_string(),
            ],
            &out,
        )
        .await;
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(!stderr.contains("must be a positive integer"), "{stderr}");

        // file_type normalization runs for valid string values.
        let (out, cap) = Output::memory();
        let _ = tools(
            "call",
            &[
                "parse_doc".to_string(),
                "--input".to_string(),
                "f.pdf".to_string(),
                "--file_type".to_string(),
                "PDF".to_string(),
            ],
            &out,
        )
        .await;
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(!stderr.contains("file_type must"), "{stderr}");

        // Non-string file_type is ignored by the normalization.
        let (out, cap) = Output::memory();
        let _ = tools(
            "call",
            &[
                "parse_doc".to_string(),
                "--input".to_string(),
                "f.pdf".to_string(),
                "--file_type".to_string(),
                "5".to_string(),
            ],
            &out,
        )
        .await;
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(!stderr.contains("file_type"), "{stderr}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tools_call_reads_args_from_stdin_child_process() {
        // Child mode: stdin is a pipe — the read path executes. Runs in the
        // same (instrumented) test binary, so its coverage merges.
        if let Some(dir) = std::env::var_os("FUTURE_CLI_STDIN_CHILD") {
            let _guard = crate::test_env::lock_env().await;
            let _env = crate::test_env::EnvGuard::set(&[("FUTURE_HOME", dir)]);
            let _keys =
                crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
            let (out, _) = Output::memory();
            let _ = tools(
                "call",
                &["web_search".to_string(), "--stdin".to_string()],
                &out,
            )
            .await;
            return;
        }
        // Parent mode: re-run THIS test in a subprocess with a piped stdin.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut child = std::process::Command::new(std::env::current_exe().expect("exe"))
            .args([
                "--exact",
                "commands::tools::tests::tools_call_reads_args_from_stdin_child_process",
                "--nocapture",
            ])
            .env("FUTURE_CLI_STDIN_CHILD", dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn child test");
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(br#"{"query":"x"}"#)
            .expect("write");
        assert!(child.wait().expect("wait").success());
    }

    #[tokio::test]
    async fn format_image_result_root_output_path_has_no_parent() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"img");
        let sc = json!({"images": [{"base64": b64}]});
        // A root output path has no parent → mkdir skipped, write fails
        // silently, and the header still prints.
        let out = format_image_result("image_gen", &sc, Some("/")).await;
        assert!(out.contains("[Image generated"), "{out}");
    }

    #[tokio::test]
    async fn tools_describe_remote_fallback() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"tools\":[{\"name\":\"remote_gem\",\"description\":\"A remote tool\"}]}}\n\n", None),
                ],
            ),
        ])
        .await;
        let _env = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        tools("describe", &["remote_gem".to_string()], &out)
            .await
            .unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("  remote_gem"), "stdout: {stdout}");
        assert!(stdout.contains("  A remote tool"), "stdout: {stdout}");
        assert!(
            stdout.contains("Remote tool — use --args with JSON"),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("future tools call remote_gem --args"),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn tools_call_validation_battery() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
        async fn run(args: &[&str]) -> (Result<(), String>, String, i32) {
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let (out, cap) = Output::memory();
            let result = tools("call", &args, &out).await;
            let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
            (result, stderr, out.exit_code())
        }

        // --help.
        let (result, _, _) = run(&["--help"]).await;
        assert!(result.is_ok());
        // Missing tool name / flag as name.
        let (result, stderr, code) = run(&[]).await;
        assert!(
            result.is_ok() && code == 1 && stderr.contains("Usage:"),
            "{stderr}"
        );
        let (result, stderr, code) = run(&["--raw"]).await;
        assert!(
            result.is_ok() && code == 1 && stderr.contains("Usage:"),
            "{stderr}"
        );

        // Required args.
        let (result, stderr, _) = run(&["search_paper"]).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(
            stderr.contains("search_paper requires: --queries"),
            "{stderr}"
        );
        assert!(
            stderr.contains("future tools describe search_paper"),
            "{stderr}"
        );

        // queries must be a non-empty array of non-empty strings.
        for bad in ["[]", "[\"  \"]", "[1]", "\"text\""] {
            let (result, stderr, _) = run(&["search_paper", "--queries", bad]).await;
            assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
            assert!(
                stderr.contains("--queries must be a non-empty array"),
                "{bad}: {stderr}"
            );
        }

        // Numeric ranges.
        let (result, stderr, _) = run(&["image_gen", "--prompt", "x", "--n", "0"]).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(
            stderr.contains("--n must be an integer between 1 and 10"),
            "{stderr}"
        );
        let (_, stderr, _) = run(&["image_gen", "--prompt", "x", "--n", "11"]).await;
        assert!(
            stderr.contains("--n must be an integer between 1 and 10"),
            "{stderr}"
        );
        let (_, stderr, _) = run(&["image_gen", "--prompt", "x", "--n", "abc"]).await;
        assert!(
            stderr.contains("--n must be an integer between 1 and 10"),
            "{stderr}"
        );
        let (_, stderr, _) = run(&["web_search", "--query", "q", "--count", "51"]).await;
        assert!(
            stderr.contains("--count must be an integer between 1 and 50"),
            "{stderr}"
        );
        let (_, stderr, _) = run(&["get_paper", "--paper_id", "PMID:1", "--max_k", "0"]).await;
        assert!(
            stderr.contains("--max_k must be a positive integer"),
            "{stderr}"
        );
        let (_, stderr, _) = run(&["read_image", "--question", "q", "--max_tokens", "-1"]).await;
        assert!(
            stderr.contains("--max_tokens must be a positive integer"),
            "{stderr}"
        );

        // file_type normalization + rejection.
        let (_, stderr, _) = run(&["parse_doc", "--file_type", "txt"]).await;
        assert!(
            stderr.contains("--file_type must be \"pdf\" or \"docx\", got: \"txt\""),
            "{stderr}"
        );

        // Negative --timeout.
        let (_, stderr, _) = run(&["web_search", "--query", "q", "--timeout", "-5"]).await;
        assert!(
            stderr.contains("--timeout must be >= 1 second, got: -5"),
            "{stderr}"
        );

        // Unreadable --input / --mask.
        let (result, stderr, _) = run(&[
            "read_image",
            "--question",
            "q",
            "--input",
            "/no/such/file.png",
        ])
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(
            stderr.contains("cannot read input file: /no/such/file.png"),
            "{stderr}"
        );
        let (result, stderr, _) =
            run(&["image_edit", "--prompt", "x", "--mask", "/no/such/mask.png"]).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(
            stderr.contains("cannot read mask file: /no/such/mask.png"),
            "{stderr}"
        );

        // Unknown tool: the catalog pre-check is skipped (the None path of
        // the `find_tool_entry` if-let) and the call falls through to the
        // remote path, failing on the missing API key.
        let (result, _, _) = run(&["no_such_tool"]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn tools_call_input_mask_base64_and_file_type_lowercasing() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Record the tool arguments the MCP server receives.
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let base = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n\n", None),
                ],
            )],
            Some(requests.clone()),
        )
        .await;
        let _env = mcp_env(&base).await;

        // parse_doc: input → doc_b64; file_type uppercased → lowercased.
        let doc = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(doc.path(), b"PDF").unwrap();
        let (out, cap) = Output::memory();
        tools(
            "call",
            &[
                "parse_doc".to_string(),
                "--input".to_string(),
                doc.path().to_str().unwrap().to_string(),
                "--file_type".to_string(),
                "PDF".to_string(),
            ],
            &out,
        )
        .await
        .expect("call");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "done\n");
        let recorded = requests.lock().unwrap().clone();
        let call = recorded
            .iter()
            .find(|r| r.contains("tools/call"))
            .expect("call");
        assert!(call.contains("doc_b64"), "call: {call}");
        assert!(call.contains("UERG"), "base64 of PDF: {call}");
        assert!(call.contains("file_type"), "call: {call}");

        // image_edit: input → image_b64, mask → mask_b64.
        let requests2 = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let base = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"content\":[]}}\n\n", None),
                ],
            )],
            Some(requests2.clone()),
        )
        .await;
        let _env = mcp_env(&base).await;
        let img = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(img.path(), b"IMG").unwrap();
        let mask = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(mask.path(), b"MSK").unwrap();
        let (out, _) = Output::memory();
        tools(
            "call",
            &[
                "image_edit".to_string(),
                "--prompt".to_string(),
                "x".to_string(),
                "--input".to_string(),
                img.path().to_str().unwrap().to_string(),
                "--mask".to_string(),
                mask.path().to_str().unwrap().to_string(),
                // --input/--mask values starting with "--" are ignored.
                "--timeout".to_string(),
                "30".to_string(),
            ],
            &out,
        )
        .await
        .expect("call");
        let recorded = requests2.lock().unwrap();
        let call = recorded
            .iter()
            .find(|r| r.contains("tools/call"))
            .expect("call");
        assert!(call.contains("image_b64"), "call: {call}");
        assert!(call.contains("mask_b64"), "call: {call}");
    }

    #[tokio::test]
    async fn tools_call_content_block_variants_and_raw() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // content blocks: text / resource / other / non-record.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"hello\"},{\"type\":\"resource\",\"resource\":{\"uri\":\"u\"}},{\"type\":\"weird\",\"x\":1},\"str\"]}}\n\n", None),
                ],
            ),
        ])
        .await;
        let _env = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        tools(
            "call",
            &[
                "web_search".to_string(),
                "--query".to_string(),
                "q".to_string(),
            ],
            &out,
        )
        .await
        .expect("call");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("hello"), "stdout: {stdout}");
        assert!(stdout.contains("\"uri\": \"u\""), "stdout: {stdout}");
        assert!(stdout.contains("\"weird\""), "stdout: {stdout}");

        // --raw with structured content → pretty JSON.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"structuredContent\":{\"query\":\"q\"},\"content\":[]}}\n\n", None),
                ],
            ),
        ])
        .await;
        let _env = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        tools(
            "call",
            &[
                "web_search".to_string(),
                "--query".to_string(),
                "q".to_string(),
                "--raw".to_string(),
            ],
            &out,
        )
        .await
        .expect("call");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("\"query\": \"q\""), "stdout: {stdout}");

        // --raw with only text → text.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    ("data: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"raw text\"}]}}\n\n", None),
                ],
            ),
        ])
        .await;
        let _env = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        tools(
            "call",
            &[
                "web_search".to_string(),
                "--query".to_string(),
                "q".to_string(),
                "--raw".to_string(),
            ],
            &out,
        )
        .await
        .expect("call");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "raw text\n");
    }

    #[tokio::test]
    async fn tools_call_error_translation_surface() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // tools/call error matching a translation.
        let base =
            crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    (
                        "data: {\"error\":{\"code\":-1,\"message\":\"insufficient_credit\"}}\n\n",
                        None,
                    ),
                ],
            )])
            .await;
        let _env = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        let err = tools(
            "call",
            &[
                "image_gen".to_string(),
                "--prompt".to_string(),
                "x".to_string(),
            ],
            &out,
        )
        .await
        .unwrap_err();
        assert_eq!(err, crate::HANDLED_EXIT);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Error: Account balance too low"),
            "stderr: {stderr}"
        );
        assert!(
            stderr.contains("Fix: Top up your account"),
            "stderr: {stderr}"
        );
        assert!(
            stderr.contains("Use \"future tools describe image_gen\" for help."),
            "stderr: {stderr}"
        );
        // Non-retryable → no retry hint.
        assert!(!stderr.contains("retry should work"), "stderr: {stderr}");

        // Retryable translation → retry hint.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    ("data: {\"error\":{\"code\":-1,\"message\":\"This operation was aborted\"}}\n\n", None),
                ],
            ),
        ])
        .await;
        let _env = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        let _ = tools(
            "call",
            &[
                "fetch_url".to_string(),
                "--url".to_string(),
                "http://x".to_string(),
            ],
            &out,
        )
        .await;
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("(This is usually temporary — retry should work.)"),
            "stderr: {stderr}"
        );

        // Untranslated error → raw message form.
        let base =
            crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse_sequence(
                "/api/v1/mcp",
                vec![
                    ("data: {\"result\":{}}\n\n", Some("s")),
                    ("data: {}\n\n", None),
                    (
                        "data: {\"error\":{\"code\":7,\"message\":\"weird failure\"}}\n\n",
                        None,
                    ),
                ],
            )])
            .await;
        let _env = mcp_env(&base).await;
        let (out, cap) = Output::memory();
        let _ = tools(
            "call",
            &[
                "web_search".to_string(),
                "--query".to_string(),
                "q".to_string(),
            ],
            &out,
        )
        .await;
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Error calling web_search: code=7, message=weird failure"),
            "stderr: {stderr}"
        );
    }

    #[tokio::test]
    async fn tools_call_auth_failure_propagates() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
        let (out, _) = Output::memory();
        let err = tools(
            "call",
            &[
                "web_search".to_string(),
                "--query".to_string(),
                "q".to_string(),
            ],
            &out,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Not logged in"), "err: {err}");
    }

    #[tokio::test]
    async fn tools_call_browser_tool_error_path() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _fh = crate::test_env::EnvGuard::temp_home();
        // Unknown browser subcommand → call_browser_tool error branch.
        let (out, cap) = Output::memory();
        let result = tools(
            "call",
            &[
                "browser".to_string(),
                "--command".to_string(),
                "bogus".to_string(),
            ],
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Unknown browser command: \"bogus\""),
            "stderr: {stderr}"
        );
    }

    // ── Remainder coverage batteries ──────────────────────────────────

    #[test]
    fn parse_tool_args_double_encoded_and_non_object() {
        // A candidate that parses to a STRING containing JSON is parsed
        // again (extra encoded layer from Windows process creation).
        let raw = r#""{\"a\":1}""#;
        let parsed = parse_tool_args(raw).unwrap();
        assert_eq!(parsed.get("a"), Some(&json!(1)));

        // A candidate parsing to a non-object (bare number) falls through
        // to the relaxed cmd-object recovery, which also fails → error.
        let err = parse_tool_args("123").unwrap_err();
        assert!(err.contains("--args must be a JSON object"), "{err}");
    }

    #[test]
    fn parse_cmd_object_rejects_malformed_fields() {
        // Field without a colon → None (relaxed path; strict JSON fails first).
        assert!(parse_tool_args("{a}").is_err());
        // Colon at position 0 → None.
        assert!(parse_tool_args("{:1}").is_err());
        // Empty key after quote-stripping → None (the second field makes the
        // strict parse fail so the relaxed path runs).
        assert!(parse_tool_args(r#"{"":1, a:2}"#).is_err());
        // Non-braced input → None from parse_cmd_object.
        assert!(parse_tool_args("nope").is_err());
    }

    #[test]
    fn parse_cmd_value_literal_edges() {
        // true / false / null.
        assert_eq!(parse_cmd_value("true"), json!(true));
        assert_eq!(parse_cmd_value("null"), Value::Null);
        // Integer and float literals.
        assert_eq!(parse_cmd_value("42"), json!(42));
        assert_eq!(parse_cmd_value("-7"), json!(-7));
        assert_eq!(parse_cmd_value("1.5"), json!(1.5));
        assert_eq!(parse_cmd_value("-2.5"), json!(-2.5));
        // Quoted strings are unwrapped.
        assert_eq!(parse_cmd_value("\"hi\""), json!("hi"));
        // Nested object and array forms.
        assert_eq!(parse_cmd_value("{a:1}"), json!({"a": 1}));
        assert_eq!(parse_cmd_value("[]"), json!([]));
        assert_eq!(parse_cmd_value("[1,x]"), json!([1, "x"]));
        // Not-quite-numbers stay strings.
        assert_eq!(parse_cmd_value("-"), json!("-"));
        assert_eq!(parse_cmd_value("1.2.3"), json!("1.2.3"));
        assert_eq!(parse_cmd_value("12a"), json!("12a"));
        assert_eq!(parse_cmd_value(""), json!(""));
    }

    #[test]
    fn literal_predicates() {
        assert!(!is_integer_literal(""));
        assert!(!is_integer_literal("-"));
        assert!(is_integer_literal("-9"));
        assert!(!is_integer_literal("1.5"));
        assert!(!is_number_literal(""));
        assert!(!is_number_literal("-"));
        assert!(!is_number_literal("1.2.3"));
        assert!(is_number_literal("-.5"));
        assert!(is_number_literal("5."));
    }

    #[test]
    fn example_flags_non_object_json() {
        assert_eq!(example_flags("[1,2]"), "");
        assert_eq!(example_flags("42"), "");
    }

    #[tokio::test]
    async fn tools_call_flag_value_edges() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
        async fn run(args: &[&str]) -> (Result<(), String>, String) {
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let (out, cap) = Output::memory();
            let result = tools("call", &args, &out).await;
            let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
            (result, stderr)
        }

        // --input / --mask immediately followed by another flag → treated
        // as absent (no value consumed).
        let (result, _stderr) = run(&["read_image", "--input", "--raw", "--prompt", "x"]).await;
        // Falls through to the API-key load (no key in the isolated HOME).
        assert!(result.is_err());

        // int_min failures.
        let (result, stderr) = run(&["search_paper", "--queries", "[\"q\"]", "--max_k", "0"]).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(
            stderr.contains("--max_k must be a positive integer"),
            "{stderr}"
        );
        let (result, stderr) = run(&["get_paper", "--paper_id", "x", "--max_tokens", "0"]).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(
            stderr.contains("--max_tokens must be a positive integer"),
            "{stderr}"
        );

        // file_type normalization: invalid value errors, valid upper-case
        // is lowercased and accepted (fails later at the API-key load).
        let (result, stderr) = run(&["parse_doc", "--doc_b64", "eA==", "--file_type", "TXT"]).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(
            stderr.contains("--file_type must be \"pdf\" or \"docx\""),
            "{stderr}"
        );
        let (result, stderr) = run(&["parse_doc", "--doc_b64", "eA==", "--file_type", "PDF"]).await;
        assert!(!stderr.contains("--file_type"), "{stderr}");
        assert!(result.is_err());

        // --timeout must be positive.
        let (result, stderr) = run(&["web_search", "--query", "q", "--timeout", "-1"]).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        assert!(stderr.contains("--timeout must be >= 1 second"), "{stderr}");
    }

    #[tokio::test]
    async fn format_image_result_defaults_and_skips() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

        // Missing size/quality/format → defaults; image_edit verb; a
        // non-record image and an image without b64 are skipped.
        let sc = json!({
            "images": [
                "junk",
                {"format": "webp"},
                {"b64_json": b64},
            ],
        });
        let result = format_image_result("image_edit", &sc, None).await;
        assert!(
            result.contains("[Image edited: unknown unknown png]"),
            "got: {result}"
        );
        assert!(result.contains("future-image-"), "got: {result}");
        assert!(result.contains(".png"), "got: {result}");

        // Header prompt line + empty images → no save section.
        let sc = json!({"size": "1x1", "quality": "hd", "format": "png", "prompt": "a fox"});
        let result = format_image_result("image_gen", &sc, None).await;
        assert!(
            result.contains("[Image generated: 1x1 hd png]"),
            "got: {result}"
        );
        assert!(result.contains("Prompt: a fox"), "got: {result}");
        assert!(!result.contains("Saved:"), "got: {result}");

        // Multi-image output path WITHOUT any dot (incl. directories) →
        // `path_N.ext`. (/tmp is dot-free; tempfile dirs are not.)
        let out = format!("/tmp/futurecli-noext-{}", std::process::id());
        let sc = json!({"images": [{"b64_json": b64}, {"b64_json": b64, "format": "jpeg"}]});
        let result = format_image_result("image_gen", &sc, Some(&out)).await;
        assert!(result.contains("noext-"), "got: {result}");
        assert!(result.contains("_1.png"), "got: {result}");
        assert!(result.contains("_2.jpg"), "got: {result}");
        let _ = std::fs::remove_file(format!("{out}_1.png"));
        let _ = std::fs::remove_file(format!("{out}_2.jpg"));

        // Default image dir + multi-image suffixes (_1/_2) with bad b64
        // skipped silently (write fails, path still listed).
        let sc = json!({"images": [{"b64_json": "!!bad!!"}, {"b64_json": b64}]});
        let result = format_image_result("image_gen", &sc, None).await;
        assert!(result.contains("future-image-"), "got: {result}");
        assert!(result.contains("_1.png"), "got: {result}");
        assert!(result.contains("_2.png"), "got: {result}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tools_call_browser_success_print_path() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_HOME",
            dir.path().as_os_str().to_os_string(),
        )]);
        // Point the browser config at a mock CDP browser, then call the
        // console command through tools_call's browser branch.
        let mock = crate::test_cdp::MockCdp::start().await;
        crate::browser::browser_state::save_browser_config(&crate::browser::types::BrowserConfig {
            version: 2,
            connection: crate::browser::types::BrowserConnectionConfig::Cdp {
                browser_kind: "chrome".to_string(),
                endpoint: mock.http_url.clone(),
            },
            ..Default::default()
        })
        .await
        .unwrap();
        let (out, cap) = Output::memory();
        tools(
            "call",
            &[
                "browser".to_string(),
                "--command".to_string(),
                "console".to_string(),
            ],
            &out,
        )
        .await
        .unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("\"logs\""), "stdout: {stdout}");
    }

    #[test]
    fn find_tool_entry_merges_browser_catalog() {
        assert!(find_tool_entry("browser").is_some());
        assert!(find_tool_entry("browser_open").is_none());
        assert!(!browser_tool_catalog().is_empty());
    }
}
