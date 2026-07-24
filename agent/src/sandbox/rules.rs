//! Path-based approval rules (APPROVAL_PLAN.md).
//!
//! Every approval decision is about a file-path access: given a path and an
//! operation (read/write), walk the rule layers top-to-bottom and return the
//! first matching verdict `Ask | Allow | Deny`. Network is unrestricted and
//! commands are not matched — only file access.
//!
//! Layers (highest priority first):
//!
//! 0. built-in security overrides — rule-file writes → deny; app credential
//!    files → read+write deny (unoverridable)
//! 1. guards — secret/credential paths (`.env`, `*.pem`, `~/.ssh` …) → ask,
//!    unoverridable by the user layers below (a broad allow can't un-gate a
//!    secret; secrets are "allow once" only)
//! 2. session — runtime "allow in this workspace/chat", current run
//! 3. workspace rule file — `${WS}/.future/approval_rule.json`
//! 4. user rule file — `~/.future/approval_rule.json`
//! - fallback: read → allow; write → in workspace/temp ? allow : ask

use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use super::paths;

/// Shared, mutable session rules — runtime "allow in this workspace/chat"
/// injections that take effect for the rest of the current run without waiting
/// for the rule file to be re-read next prompt.
pub type SessionRules = Arc<Mutex<Vec<PathRule>>>;

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
    pub fn parse(value: &str) -> Self {
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
///   (auth.json is TEMPORARILY allowed for testing — see the block below;
///   the hard-deny blocks the official `future` CLI used by skills.
///   models.json stays denied.)
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
        // NOTE: auth.json is TEMPORARILY allowed (omitted from the deny list
        // below). Re-add it once the trusted-CLI credential-access story is
        // designed.
        //
        // Background: skills sometimes shell out to our official `future` CLI,
        // which legitimately reads `~/.future/agent/auth.json`. With a hard-deny
        // in place, `future` is blocked inside the Seatbelt sandbox (this
        // override is layer-0, unoverridable by any user approval rule), so
        // those skill flows fail during testing.
        //
        // We WANT to trust `future` specifically without opening auth.json to
        // arbitrary shell commands — but a shared sandbox can't distinguish
        // `future` from a sibling `cat` in the same command, so per-binary
        // trust isn't expressible here. The proper fix is a dedicated
        // credential channel (agent injects a short-lived scoped token via env,
        // or `future` reverse-requests the key from the agent over a socket
        // with peer-credential verification), not a path allow-hole. That's a
        // larger, cross-platform effort — deferred.
        //
        // NOTE: while auth.json is allowed, any shell command can read/write it
        // — acceptable for local testing only. models.json stays denied.
        for cred in [
            // ".future/agent/auth.json",      // TEMPORARILY allowed — see above
            ".future/agent/models.json",
            // ".future/agent-app/auth.json",  // TEMPORARILY allowed — see above
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

/// Layer 1 (guards): credential/secret paths → ask. These sit ABOVE the user
/// rule files, so a broad allow (`src/config/*`) can never silently un-gate a
/// secret that lands in that directory. Secrets are therefore "allow once"
/// only — never persistently allowed (a deliberate safety/simplicity choice,
/// APPROVAL_PLAN.md §3). Temp dirs are NOT here — they're part of the writable
/// fallback, so they never shadow a secret.
pub fn builtin_guards(workspace: &Path, home: Option<&Path>) -> Vec<PathRule> {
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
    // `mut` is only exercised by the `#[cfg(unix)]` push below; on Windows that
    // block is compiled out, so silence the otherwise-unused `mut` there.
    #[cfg_attr(not(unix), allow(unused_mut))]
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
/// Priority (highest first): overrides > guards > session > workspace > user >
/// fallback. Guards (secrets) sit above the user layers so they can't be
/// overridden by a broad allow.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    pub workspace: PathBuf,
    /// Canonicalized temp roots — writable via the fallback (not a rule, so
    /// they can't shadow a secret ask).
    temp_roots: Vec<PathBuf>,
    overrides: Vec<PathRule>,
    guards: Vec<PathRule>,
    session: SessionRules,
    workspace_rules: Vec<PathRule>,
    user_rules: Vec<PathRule>,
}

impl RuleSet {
    pub fn resolve(workspace: &Path) -> Self {
        Self::resolve_with_session(workspace, Arc::new(Mutex::new(vec![])))
    }

    /// Resolve, sharing `session` so runtime injections (same-run "allow in
    /// this workspace") are visible to the live sandbox.
    pub fn resolve_with_session(workspace: &Path, session: SessionRules) -> Self {
        let home = dirs::home_dir();
        let user_rule_file = home.as_ref().map(|h| h.join(".future/approval_rule.json"));
        Self::resolve_impl(
            workspace,
            home.as_deref(),
            user_rule_file.as_deref(),
            session,
        )
    }

    /// Test-only constructor: uses the real home for built-in guards (tests
    /// assert on paths like `~/.ssh/id_rsa`), but points the user-rule layer
    /// at a nonexistent file so the developer machine's real
    /// `~/.future/approval_rule.json` can never leak into a test outcome.
    /// Reading mutable machine-global state made these tests flaky.
    #[cfg(test)]
    fn resolve_isolated(workspace: &Path) -> Self {
        let home = dirs::home_dir();
        let stub = workspace.join(".future/user-rules-that-do-not-exist.json");
        Self::resolve_impl(
            workspace,
            home.as_deref(),
            Some(&stub),
            Arc::new(Mutex::new(vec![])),
        )
    }

    /// Full constructor with an injectable user-rule file path.
    fn resolve_impl(
        workspace: &Path,
        home: Option<&Path>,
        user_rule_file: Option<&Path>,
        session: SessionRules,
    ) -> Self {
        let workspace = paths::canonicalize_lenient(workspace);

        let workspace_rules =
            load_rule_file(&workspace.join(".future/approval_rule.json"), &workspace)
                .unwrap_or_else(|error| {
                    tracing::warn!("{error}");
                    vec![]
                });
        let user_rules = match user_rule_file {
            Some(file) => load_rule_file(file, &workspace).unwrap_or_else(|error| {
                tracing::warn!("{error}");
                vec![]
            }),
            None => vec![],
        };

        Self {
            temp_roots: temp_roots(),
            overrides: builtin_overrides(&workspace, home),
            guards: builtin_guards(&workspace, home),
            session,
            workspace_rules,
            user_rules,
            workspace,
        }
    }

    /// Add a runtime allow rule (same-run "allow in this workspace/chat").
    pub fn add_session_rule(&self, abs_pattern: &str, access: Access, decision: Decision) {
        self.session
            .lock()
            .push(PathRule::new(abs_pattern, access, decision));
    }

    /// Whether `path` matches a built-in secret guard (either op). Used to
    /// suppress the "allow in this workspace" persistence for secret files.
    pub fn is_secret_path(&self, path: &Path) -> bool {
        self.guards
            .iter()
            .any(|rule| rule.matches(path, Op::Read) || rule.matches(path, Op::Write))
    }

    /// Evaluate a file access. `path` should already be canonicalized by the
    /// caller (tools canonicalize before calling).
    pub fn evaluate(&self, path: &Path, op: Op) -> Decision {
        let session = self.session.lock();
        let session_slice: &[PathRule] = session.as_slice();
        for layer in [
            self.overrides.as_slice(),
            self.guards.as_slice(),
            session_slice,
            self.workspace_rules.as_slice(),
            self.user_rules.as_slice(),
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

    /// All rule layers in priority order (highest first), snapshotted for the
    /// Seatbelt profile builder (session is cloned under its lock).
    pub fn profile_layers(&self) -> Vec<Vec<PathRule>> {
        let session = self.session.lock().clone();
        vec![
            self.overrides.clone(),
            self.guards.clone(),
            session,
            self.workspace_rules.clone(),
            self.user_rules.clone(),
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

/// Push a runtime allow rule into a shared session-rules handle, resolving
/// `raw_pattern` against `workspace` (same rules as file entries). Used by the
/// RPC layer for same-run "allow in this workspace/chat".
pub fn push_session_allow(
    session: &SessionRules,
    workspace: &Path,
    raw_pattern: &str,
    access: Access,
) {
    session.lock().push(PathRule::new(
        &absolutize(workspace, raw_pattern),
        access,
        Decision::Allow,
    ));
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
        let set = RuleSet::resolve_isolated(&workspace);
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
        let set = RuleSet::resolve_isolated(&workspace);
        let tmp = paths::canonicalize_lenient(&std::env::temp_dir()).join("futureos-t.txt");
        assert_eq!(set.evaluate(&tmp, Op::Write), Decision::Allow);
        assert_eq!(set.evaluate(&tmp, Op::Read), Decision::Allow);
    }

    #[test]
    fn credential_reads_ask() {
        let workspace = ws();
        let set = RuleSet::resolve_isolated(&workspace);
        let ssh = dirs::home_dir().unwrap().join(".ssh/id_rsa");
        assert_eq!(set.evaluate(&ssh, Op::Read), Decision::Ask);
    }

    #[test]
    fn workspace_env_asks_even_though_in_workspace() {
        let workspace = ws();
        let set = RuleSet::resolve_isolated(&workspace);
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
        let set = RuleSet::resolve_isolated(&workspace);
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
    fn secret_guard_is_unoverridable_by_user_rule() {
        // Plan A: a workspace allow for `.env` must NOT lift the secret guard.
        let workspace = ws();
        std::fs::create_dir_all(workspace.join(".future")).unwrap();
        std::fs::write(
            workspace.join(".future/approval_rule.json"),
            r#"{"rules":[{"path":".env","access":"read","action":"allow"}]}"#,
        )
        .unwrap();
        let set = RuleSet::resolve_isolated(&workspace);
        assert_eq!(
            set.evaluate(&workspace.join(".env"), Op::Read),
            Decision::Ask
        );
    }

    #[test]
    fn broad_allow_does_not_ungate_secret_in_dir() {
        // The exact Q1 case: `config/*` allow must not un-gate a `.pem` inside.
        let workspace = ws();
        std::fs::create_dir_all(workspace.join(".future")).unwrap();
        std::fs::write(
            workspace.join(".future/approval_rule.json"),
            r#"{"rules":[{"path":"config/*","access":"write","action":"allow"}]}"#,
        )
        .unwrap();
        let set = RuleSet::resolve_isolated(&workspace);
        assert_eq!(
            set.evaluate(&workspace.join("config/app.yaml"), Op::Write),
            Decision::Allow
        );
        assert_eq!(
            set.evaluate(&workspace.join("config/tls.pem"), Op::Write),
            Decision::Ask
        );
        assert!(set.is_secret_path(&workspace.join("config/tls.pem")));
        assert!(!set.is_secret_path(&workspace.join("config/app.yaml")));
    }

    #[test]
    fn session_allow_ungates_non_secret_only() {
        let workspace = ws();
        let set = RuleSet::resolve_isolated(&workspace);
        let outside = dirs::home_dir().unwrap().join("futureos-session-dir");
        set.add_session_rule(
            &outside.join("*").to_string_lossy(),
            Access::Write,
            Decision::Allow,
        );
        assert_eq!(
            set.evaluate(&outside.join("note.txt"), Op::Write),
            Decision::Allow
        );
        // A session allow can't lift a secret guard.
        set.add_session_rule(
            &workspace.join("*").to_string_lossy(),
            Access::Read,
            Decision::Allow,
        );
        assert_eq!(
            set.evaluate(&workspace.join(".env"), Op::Read),
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
        let set = RuleSet::resolve_isolated(&workspace);
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
        let set = RuleSet::resolve_isolated(&workspace);
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
        let set = RuleSet::resolve_isolated(&workspace);
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
