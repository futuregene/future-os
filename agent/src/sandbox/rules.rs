//! Path-based approval rules (APPROVAL_PLAN.md).
//!
//! Every approval decision is about a file-path access: given a path and an
//! operation (read/write), walk the rule layers top-to-bottom and return the
//! first matching verdict `Ask | Allow | Deny`. Network is unrestricted and
//! commands are not matched — only file access.
//!
//! Layers (highest priority first):
//!
//! - 0. built-in security overrides — rule-file writes → deny; app credential
//!   files → read+write deny (unoverridable)
//! - 1. session (runtime "allow in this workspace", current run)
//! - 2. workspace rule file — `${WS}/.future/approval_rule.json`
//! - 3. user rule file — `~/.future/approval_rule.json`
//! - 4. built-in defaults — credential reads → ask
//! - fallback: read → allow; write → in workspace/temp ? allow : ask

use std::path::{Path, PathBuf};

use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use super::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Both,
}

impl Access {
    fn parse(value: &str) -> Self {
        match value {
            "read" => Self::Read,
            "write" => Self::Write,
            _ => Self::Both,
        }
    }

    fn covers(&self, op: Op) -> bool {
        matches!(
            (self, op),
            (Access::Both, _) | (Access::Read, Op::Read) | (Access::Write, Op::Write)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Ask,
    Allow,
    Deny,
}

impl Decision {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(Self::Ask),
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// A compiled path matcher. No-wildcard patterns match the path itself and its
/// whole subtree (`~/.ssh` covers `~/.ssh` and everything under it); wildcard
/// patterns match by glob (`*` within a segment, `**` across segments, `?` one
/// char).
#[derive(Debug, Clone)]
enum Matcher {
    /// Canonicalized base; matches the base or anything under it.
    Subtree(PathBuf),
    /// Anchored regex over the canonicalized path string.
    Glob(Regex),
}

#[derive(Debug, Clone)]
pub struct PathRule {
    matcher: Matcher,
    access: Access,
    decision: Decision,
}

impl PathRule {
    fn matches(&self, path: &Path, op: Op) -> bool {
        if !self.access.covers(op) {
            return false;
        }
        match &self.matcher {
            Matcher::Subtree(base) => paths::path_within(path, base),
            Matcher::Glob(re) => re.is_match(&path.to_string_lossy()),
        }
    }

    /// Build a rule from an already absolute pattern (tilde/relative resolved).
    fn new(abs_pattern: &str, access: Access, decision: Decision) -> Self {
        Self {
            matcher: compile_matcher(abs_pattern),
            access,
            decision,
        }
    }
}

/// Resolve `raw` to an absolute pattern: expand `~`, and join a relative
/// pattern onto `base` (the workspace root). Glob metacharacters are preserved.
fn absolutize(base: &Path, raw: &str) -> String {
    let expanded = paths::expand_tilde(raw);
    if expanded.is_absolute() {
        expanded.to_string_lossy().into_owned()
    } else {
        base.join(expanded).to_string_lossy().into_owned()
    }
}

fn has_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

fn compile_matcher(abs_pattern: &str) -> Matcher {
    if !has_glob(abs_pattern) {
        return Matcher::Subtree(paths::canonicalize_lenient(Path::new(abs_pattern)));
    }
    // Canonicalize the leading non-glob prefix (symlink-correct), keep the
    // globbed remainder verbatim, then compile to an anchored regex.
    let segments: Vec<&str> = abs_pattern.split('/').collect();
    let mut prefix = PathBuf::from("/");
    let mut split_at = segments.len();
    for (idx, seg) in segments.iter().enumerate() {
        if has_glob(seg) {
            split_at = idx;
            break;
        }
        if !seg.is_empty() {
            prefix.push(seg);
        }
    }
    let canon_prefix = paths::canonicalize_lenient(&prefix);
    let rest = segments[split_at..].join("/");
    let full = format!(
        "{}/{}",
        canon_prefix.to_string_lossy().trim_end_matches('/'),
        rest
    );
    match build_glob_regex(&full) {
        Some(re) => Matcher::Glob(re),
        // A pattern that fails to compile matches nothing (fail-safe: it won't
        // silently widen access).
        None => Matcher::Glob(Regex::new("$^").unwrap()),
    }
}

/// Convert a glob to an anchored regex. `**` matches across `/`, `*` within a
/// segment, `?` one non-`/` char. Case-insensitive on macOS (APFS default).
fn build_glob_regex(glob: &str) -> Option<Regex> {
    let mut re = String::from("^");
    let bytes = glob.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '*' => {
                if i + 1 < bytes.len() && bytes[i + 1] as char == '*' {
                    re.push_str(".*");
                    i += 2;
                    // Collapse `/**/` so `a/**/b` also matches `a/b`.
                    if i < bytes.len() && bytes[i] as char == '/' {
                        i += 1;
                    }
                    continue;
                }
                re.push_str("[^/]*");
            }
            '?' => re.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            _ => re.push(c),
        }
        i += 1;
    }
    re.push('$');
    RegexBuilder::new(&re)
        .case_insensitive(cfg!(target_os = "macos"))
        .build()
        .ok()
}

// ─── Rule file parsing ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RuleFile {
    #[serde(default)]
    rules: Vec<RawRule>,
}

#[derive(Debug, Deserialize)]
struct RawRule {
    path: Option<String>,
    #[serde(default)]
    access: Option<String>,
    action: Option<String>,
}

/// Parse a rule file's JSON into rules resolved against `workspace`. Returns
/// `None` (and lets the caller log/skip the layer) when the file is missing or
/// malformed — never fails the run, never fails open.
pub fn parse_rule_file(contents: &str, workspace: &Path) -> Option<Vec<PathRule>> {
    let parsed: RuleFile = serde_json::from_str(contents).ok()?;
    let rules = parsed
        .rules
        .into_iter()
        .filter_map(|raw| {
            let path = raw.path?;
            let decision = Decision::parse(raw.action.as_deref().unwrap_or(""))?;
            let access = Access::parse(raw.access.as_deref().unwrap_or("both"));
            Some(PathRule::new(
                &absolutize(workspace, &path),
                access,
                decision,
            ))
        })
        .collect();
    Some(rules)
}

/// Load and parse a rule file from disk. Missing file → empty (no rules).
/// Present-but-broken → `Err` with the reason (caller logs + skips the layer).
pub fn load_rule_file(path: &Path, workspace: &Path) -> Result<Vec<PathRule>, String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => parse_rule_file(&contents, workspace)
            .ok_or_else(|| format!("malformed rule file: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(error) => Err(format!("unreadable rule file {}: {error}", path.display())),
    }
}

// ─── Built-in layers ────────────────────────────────────────────────────────

/// Layer 0: security overrides, unoverridable by any user layer.
/// - Rule files: WRITE denied (so the agent can't rewrite its own rules to
///   escalate) but readable.
/// - The app's own credential/config files: READ+WRITE denied — the agent has
///   no legitimate reason to touch its own API keys / provider configs.
pub fn builtin_overrides(workspace: &Path, home: Option<&Path>) -> Vec<PathRule> {
    let mut rules = vec![PathRule::new(
        &workspace
            .join(".future/approval_rule.json")
            .to_string_lossy(),
        Access::Write,
        Decision::Deny,
    )];
    if let Some(home) = home {
        rules.push(PathRule::new(
            &home.join(".future/approval_rule.json").to_string_lossy(),
            Access::Write,
            Decision::Deny,
        ));
        for cred in [
            ".future/agent/auth.json",
            ".future/agent/models.json",
            ".future/agent-app/auth.json",
            ".future/agent-app/models.json",
        ] {
            rules.push(PathRule::new(
                &home.join(cred).to_string_lossy(),
                Access::Both,
                Decision::Deny,
            ));
        }
    }
    rules
}

/// Layer 4 (built-in defaults): credential/secret reads-and-writes → ask.
/// Temp dirs are NOT here — they're part of the writable fallback, so they
/// never shadow a secret rule. Overridable by the user layers above.
pub fn builtin_defaults(workspace: &Path, home: Option<&Path>) -> Vec<PathRule> {
    let mut rules = Vec::new();

    // Home-level credential / privacy paths → ask.
    if let Some(home) = home {
        const HOME_SECRETS: &[&str] = &[
            ".ssh",
            ".gnupg",
            ".npmrc",
            ".pypirc",
            ".cargo/credentials",
            ".cargo/credentials.toml",
            ".gem/credentials",
            ".netrc",
            ".git-credentials",
            ".env",
            ".aws",
            ".azure",
            ".config/gcloud",
            ".terraform.d",
            ".kube/config",
            ".docker/config.json",
            ".config/gh",
            "Library/Keychains",
        ];
        for rel in HOME_SECRETS {
            rules.push(PathRule::new(
                &home.join(rel).to_string_lossy(),
                Access::Both,
                Decision::Ask,
            ));
        }
    }

    // Workspace-internal secrets → ask (glob, relative to workspace).
    const WORKSPACE_SECRETS: &[&str] = &[
        ".env",
        ".env.*",
        "**/*.pem",
        "**/*.key",
        "**/*.p12",
        "**/id_rsa*",
    ];
    for pat in WORKSPACE_SECRETS {
        rules.push(PathRule::new(
            &absolutize(workspace, pat),
            Access::Both,
            Decision::Ask,
        ));
    }

    rules
}

/// Canonicalized temp roots ($TMPDIR + /tmp).
pub fn temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![paths::canonicalize_lenient(&std::env::temp_dir())];
    #[cfg(unix)]
    {
        let slash_tmp = paths::canonicalize_lenient(Path::new("/tmp"));
        if !roots.contains(&slash_tmp) {
            roots.push(slash_tmp);
        }
    }
    roots
}

// ─── Rule set + evaluation ──────────────────────────────────────────────────

/// The fully-resolved, ordered rule layers for one session/workspace.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    pub workspace: PathBuf,
    /// Canonicalized temp roots — writable via the fallback (not a rule, so
    /// they can't shadow a secret ask).
    temp_roots: Vec<PathBuf>,
    overrides: Vec<PathRule>,
    session: Vec<PathRule>,
    workspace_rules: Vec<PathRule>,
    user_rules: Vec<PathRule>,
    defaults: Vec<PathRule>,
}

impl RuleSet {
    pub fn resolve(workspace: &Path) -> Self {
        let workspace = paths::canonicalize_lenient(workspace);
        let home = dirs::home_dir();
        let home_ref = home.as_deref();

        let workspace_rules =
            load_rule_file(&workspace.join(".future/approval_rule.json"), &workspace)
                .unwrap_or_else(|error| {
                    tracing::warn!("{error}");
                    vec![]
                });
        let user_rules = match home_ref {
            Some(home) => load_rule_file(&home.join(".future/approval_rule.json"), &workspace)
                .unwrap_or_else(|error| {
                    tracing::warn!("{error}");
                    vec![]
                }),
            None => vec![],
        };

        Self {
            temp_roots: temp_roots(),
            overrides: builtin_overrides(&workspace, home_ref),
            session: vec![],
            workspace_rules,
            user_rules,
            defaults: builtin_defaults(&workspace, home_ref),
            workspace,
        }
    }

    /// Add a runtime "allow in this workspace" rule that takes effect for the
    /// rest of the current run (before the file re-read next prompt).
    pub fn add_session_rule(&mut self, abs_pattern: &str, access: Access, decision: Decision) {
        self.session
            .push(PathRule::new(abs_pattern, access, decision));
    }

    /// Evaluate a file access. `path` should already be canonicalized by the
    /// caller (tools canonicalize before calling).
    pub fn evaluate(&self, path: &Path, op: Op) -> Decision {
        for layer in [
            &self.overrides,
            &self.session,
            &self.workspace_rules,
            &self.user_rules,
            &self.defaults,
        ] {
            for rule in layer {
                if rule.matches(path, op) {
                    return rule.decision;
                }
            }
        }
        // Fallback: reads open; writes allowed inside the workspace or temp.
        match op {
            Op::Read => Decision::Allow,
            Op::Write => {
                let writable = paths::path_within(path, &self.workspace)
                    || self.temp_roots.iter().any(|r| paths::path_within(path, r));
                if writable {
                    Decision::Allow
                } else {
                    Decision::Ask
                }
            }
        }
    }

    /// All rule layers in priority order (highest first), for the Seatbelt
    /// profile builder to translate into SBPL allow/deny clauses.
    pub fn layers(&self) -> [&[PathRule]; 5] {
        [
            &self.overrides,
            &self.session,
            &self.workspace_rules,
            &self.user_rules,
            &self.defaults,
        ]
    }
}

impl PathRule {
    pub fn access(&self) -> Access {
        self.access
    }

    pub fn decision(&self) -> Decision {
        self.decision
    }

    /// The matcher as an SBPL filter fragment source: either a canonicalized
    /// subtree base, or the original glob's compiled regex source.
    pub fn matcher_sbpl(&self) -> MatcherSbpl<'_> {
        match &self.matcher {
            Matcher::Subtree(base) => MatcherSbpl::Subtree(base),
            Matcher::Glob(re) => MatcherSbpl::Regex(re.as_str()),
        }
    }
}

pub enum MatcherSbpl<'a> {
    Subtree(&'a Path),
    Regex(&'a str),
}

impl Access {
    pub fn covers_read(&self) -> bool {
        matches!(self, Access::Read | Access::Both)
    }
    pub fn covers_write(&self) -> bool {
        matches!(self, Access::Write | Access::Both)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("futureos-rules-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        paths::canonicalize_lenient(&dir)
    }

    #[test]
    fn fallback_reads_open_writes_gated() {
        let workspace = ws();
        let set = RuleSet::resolve(&workspace);
        // Read anywhere → allow.
        assert_eq!(
            set.evaluate(Path::new("/usr/lib/x"), Op::Read),
            Decision::Allow
        );
        // Write in workspace → allow.
        assert_eq!(
            set.evaluate(&workspace.join("src/main.rs"), Op::Write),
            Decision::Allow
        );
        // Write outside → ask.
        let outside = dirs::home_dir().unwrap().join("futureos-outside-xyz.txt");
        assert_eq!(set.evaluate(&outside, Op::Write), Decision::Ask);
    }

    #[test]
    fn temp_is_allowed_read_and_write() {
        let workspace = ws();
        let set = RuleSet::resolve(&workspace);
        let tmp = paths::canonicalize_lenient(&std::env::temp_dir()).join("futureos-t.txt");
        assert_eq!(set.evaluate(&tmp, Op::Write), Decision::Allow);
        assert_eq!(set.evaluate(&tmp, Op::Read), Decision::Allow);
    }

    #[test]
    fn credential_reads_ask() {
        let workspace = ws();
        let set = RuleSet::resolve(&workspace);
        let ssh = dirs::home_dir().unwrap().join(".ssh/id_rsa");
        assert_eq!(set.evaluate(&ssh, Op::Read), Decision::Ask);
    }

    #[test]
    fn workspace_env_asks_even_though_in_workspace() {
        let workspace = ws();
        let set = RuleSet::resolve(&workspace);
        // .env would be write-allowed by fallback, but layer-4 ask wins first.
        assert_eq!(
            set.evaluate(&workspace.join(".env"), Op::Read),
            Decision::Ask
        );
        assert_eq!(
            set.evaluate(&workspace.join(".env"), Op::Write),
            Decision::Ask
        );
        assert_eq!(
            set.evaluate(&workspace.join("config/db.key"), Op::Read),
            Decision::Ask
        );
    }

    #[test]
    fn rule_file_write_is_denied_and_unoverridable() {
        let workspace = ws();
        let mut set = RuleSet::resolve(&workspace);
        let rulefile = workspace.join(".future/approval_rule.json");
        assert_eq!(set.evaluate(&rulefile, Op::Write), Decision::Deny);
        // Even a session allow can't override the layer-0 deny.
        set.add_session_rule(
            &workspace.join(".future/**").to_string_lossy(),
            Access::Write,
            Decision::Allow,
        );
        assert_eq!(set.evaluate(&rulefile, Op::Write), Decision::Deny);
        // But reading the rule file is fine.
        assert_eq!(set.evaluate(&rulefile, Op::Read), Decision::Allow);
    }

    #[test]
    fn workspace_file_rule_overrides_default() {
        let workspace = ws();
        std::fs::create_dir_all(workspace.join(".future")).unwrap();
        std::fs::write(
            workspace.join(".future/approval_rule.json"),
            r#"{"version":1,"rules":[{"path":".env","access":"read","action":"allow"}]}"#,
        )
        .unwrap();
        let set = RuleSet::resolve(&workspace);
        // Workspace rule (layer 2) beats the built-in .env ask (layer 4).
        assert_eq!(
            set.evaluate(&workspace.join(".env"), Op::Read),
            Decision::Allow
        );
        // Write still asks (rule was read-only).
        assert_eq!(
            set.evaluate(&workspace.join(".env"), Op::Write),
            Decision::Ask
        );
    }

    #[test]
    fn no_wildcard_matches_subtree() {
        let workspace = ws();
        std::fs::create_dir_all(workspace.join(".future")).unwrap();
        std::fs::write(
            workspace.join(".future/approval_rule.json"),
            r#"{"rules":[{"path":"vendor","action":"deny"}]}"#,
        )
        .unwrap();
        let set = RuleSet::resolve(&workspace);
        assert_eq!(
            set.evaluate(&workspace.join("vendor"), Op::Read),
            Decision::Deny
        );
        assert_eq!(
            set.evaluate(&workspace.join("vendor/lib/x.rs"), Op::Read),
            Decision::Deny
        );
        assert_eq!(
            set.evaluate(&workspace.join("vendored.txt"), Op::Read),
            Decision::Allow
        );
    }

    #[test]
    fn glob_matches_by_segment() {
        // Use deny so the glob actually changes the verdict (in-workspace
        // writes are allowed by the fallback otherwise).
        let workspace = ws();
        std::fs::create_dir_all(workspace.join(".future")).unwrap();
        std::fs::write(
            workspace.join(".future/approval_rule.json"),
            r#"{"rules":[{"path":"build/*","access":"write","action":"deny"}]}"#,
        )
        .unwrap();
        let set = RuleSet::resolve(&workspace);
        assert_eq!(
            set.evaluate(&workspace.join("build/out.o"), Op::Write),
            Decision::Deny
        );
        // `build/*` does not cross a slash → falls to fallback (in workspace → allow).
        assert_eq!(
            set.evaluate(&workspace.join("build/sub/out.o"), Op::Write),
            Decision::Allow
        );
    }

    #[test]
    fn malformed_file_is_skipped_not_fatal() {
        let workspace = ws();
        std::fs::create_dir_all(workspace.join(".future")).unwrap();
        std::fs::write(workspace.join(".future/approval_rule.json"), "{ not json").unwrap();
        // resolve() logs + skips; built-ins + fallback still apply.
        let set = RuleSet::resolve(&workspace);
        assert_eq!(
            set.evaluate(&workspace.join("a.txt"), Op::Read),
            Decision::Allow
        );
        let ssh = dirs::home_dir().unwrap().join(".ssh/x");
        assert_eq!(set.evaluate(&ssh, Op::Read), Decision::Ask);
    }

    #[test]
    fn parse_ignores_bad_rules_keeps_good() {
        let workspace = ws();
        let rules = parse_rule_file(
            r#"{"rules":[
                {"path":"a","action":"allow"},
                {"action":"deny"},
                {"path":"b","action":"nonsense"},
                {"path":"c","access":"read","action":"deny"}
            ]}"#,
            &workspace,
        )
        .unwrap();
        assert_eq!(rules.len(), 2); // only "a" and "c" are valid
    }
}
