//! CLI command registration framework (G-26) — LoopX `cli.py` + the
//! `register_*_commands()` pattern, natively.
//!
//! The registry is the ecosystem injection point: a command group is
//! declared once, commands register into it with a one-line summary and a
//! usage line, and `--help` aggregates everything by group instead of a
//! hard-coded help string.
//!
//! Dispatch stays in the binary: the registry validates `args[0]` against the
//! registered command set (unknown command → error with a hint) and the
//! binary routes to its handler. Handlers keep their heterogeneous signatures
//! (`&Store` / `&mut Store` / async) — the registry does not force a uniform
//! handler type, which keeps the migration a pure assembly change.

/// Operator journey metadata (P1-9) — the role-based lens used by
/// `future loop commands`. LoopX presents its CLI in five operator groups
/// (Start here / Daily operator / Loop driver / Setup & automation /
/// Maintainer & adapter); the registry itself stays the flat machine
/// catalog (`future loop registry`), journeys are a pure presentation
/// overlay on top of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Journey {
    Starter,
    Daily,
    Driver,
    Setup,
    Maintainer,
}

impl Journey {
    /// Display order for the grouped command reference.
    pub const ALL: [Journey; 5] = [
        Journey::Starter,
        Journey::Daily,
        Journey::Driver,
        Journey::Setup,
        Journey::Maintainer,
    ];

    /// Stable machine key (JSON output).
    pub fn key(self) -> &'static str {
        match self {
            Journey::Starter => "starter",
            Journey::Daily => "daily",
            Journey::Driver => "driver",
            Journey::Setup => "setup",
            Journey::Maintainer => "maintainer",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Journey::Starter => "Start here",
            Journey::Daily => "Daily operator",
            Journey::Driver => "Loop driver",
            Journey::Setup => "Setup & automation",
            Journey::Maintainer => "Maintainer & adapter",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Journey::Starter => "create the first goal, read status, check the install",
            Journey::Daily => {
                "todos, gates, replans, leases, quota — the day-to-day control surface"
            }
            Journey::Driver => "per-turn loop execution: run envelopes, heartbeats, agent lanes",
            Journey::Setup => "one-time configuration: authority, profiles, automation",
            Journey::Maintainer => {
                "quality gates (benchmark/canary/replay), retention, introspection"
            }
        }
    }
}

/// One registered command.
#[derive(Debug, Clone)]
pub struct CommandDef {
    pub name: String,
    pub summary: String,
    pub usage: String,
    /// True for commands surfaced only with `--include-experimental`.
    pub experimental: bool,
    /// Operator journey (P1-9). Defaults to maintainer; the CLI builder
    /// reassigns statically known commands via `set_journey`.
    pub journey: Journey,
    /// Per-subcommand usage for multi-verb commands (`supervisor steer`,
    /// `todo update`, `worker stop`, …). Empty for single-verb commands.
    /// Rendered by `<command> --help` and addressable via `<command> <sub>
    /// --help` so an AI orchestrator can discover the exact flags of one
    /// verb instead of a merged top-level usage line.
    pub subcommands: Vec<CommandDef>,
}

/// One registered command group (LoopX command groups: goal / todo / agent /
/// ops / multi-agent / work-items / cli).
#[derive(Debug, Clone)]
pub struct GroupDef {
    pub name: String,
    pub summary: String,
}

/// The command registry: ordered groups + ordered commands per group.
/// Ordered so `--help` renders in registration order (stable output).
#[derive(Debug, Clone, Default)]
pub struct CommandRegistry {
    groups: Vec<GroupDef>,
    /// (group index, command)
    commands: Vec<(usize, CommandDef)>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a group (idempotent by name). Returns its index.
    pub fn group(&mut self, name: &str, summary: &str) -> usize {
        if let Some(idx) = self.groups.iter().position(|g| g.name == name) {
            return idx;
        }
        self.groups.push(GroupDef {
            name: name.to_string(),
            summary: summary.to_string(),
        });
        self.groups.len() - 1
    }

    /// Register a command into a group. `usage` is the full usage line
    /// (subcommand flags), `summary` the one-line description.
    pub fn command(&mut self, group: usize, name: &str, summary: &str, usage: &str) -> &mut Self {
        self.register_command(group, name, summary, usage, false)
    }

    /// Register a command with an explicit experimental flag (G-24: hidden
    /// unless `--include-experimental`).
    pub fn command_experimental(
        &mut self,
        group: usize,
        name: &str,
        summary: &str,
        usage: &str,
    ) -> &mut Self {
        self.register_command(group, name, summary, usage, true)
    }

    fn register_command(
        &mut self,
        group: usize,
        name: &str,
        summary: &str,
        usage: &str,
        experimental: bool,
    ) -> &mut Self {
        // Idempotent: re-registering the same name in the same group is a no-op.
        let already = self
            .commands
            .iter()
            .any(|(g, c)| *g == group && c.name == name);
        if !already {
            self.commands.push((
                group,
                CommandDef {
                    name: name.to_string(),
                    summary: summary.to_string(),
                    usage: usage.to_string(),
                    experimental,
                    journey: Journey::Maintainer,
                    subcommands: Vec::new(),
                },
            ));
        }
        self
    }

    /// Register a subcommand (verb) under an already-registered command so
    /// `<command> --help` lists it and `<command> <sub> --help` renders its
    /// exact usage. No-op when the parent command is unknown or the
    /// subcommand name is already present (idempotent).
    pub fn subcommand(
        &mut self,
        command: &str,
        name: &str,
        summary: &str,
        usage: &str,
    ) -> &mut Self {
        for (_, c) in &mut self.commands {
            if c.name == command && !c.subcommands.iter().any(|s| s.name == name) {
                c.subcommands.push(CommandDef {
                    name: name.to_string(),
                    summary: summary.to_string(),
                    usage: usage.to_string(),
                    experimental: false,
                    journey: Journey::Maintainer,
                    subcommands: Vec::new(),
                });
            }
        }
        self
    }

    /// Find a subcommand by `parent` + `sub` name.
    pub fn find_subcommand(
        &self,
        command: &str,
        sub: &str,
        include_experimental: bool,
    ) -> Option<(&GroupDef, &CommandDef, &CommandDef)> {
        let (group, parent) = self.find(command, include_experimental)?;
        let sub_def = parent.subcommands.iter().find(|s| s.name == sub)?;
        Some((group, parent, sub_def))
    }

    /// Assign the operator journey (P1-9) for an already-registered
    /// command. No-op for unknown names.
    pub fn set_journey(&mut self, command: &str, journey: Journey) -> &mut Self {
        for (_, c) in &mut self.commands {
            if c.name == command {
                c.journey = journey;
            }
        }
        self
    }

    /// Commands of one journey (registration order).
    pub fn commands_in_journey(
        &self,
        journey: Journey,
        include_experimental: bool,
    ) -> Vec<&CommandDef> {
        self.commands
            .iter()
            .filter(|(_, c)| c.journey == journey && (include_experimental || !c.experimental))
            .map(|(_, c)| c)
            .collect()
    }

    /// P1-9: the grouped operator command reference (`future loop
    /// commands`) — the five journey sections in display order, usage +
    /// summary per command (LoopX `loopx commands` presentation).
    pub fn render_journeys(&self, include_experimental: bool) -> String {
        let mut out = String::new();
        out.push_str("FutureOS loop command reference — grouped by operator journey\n\n");
        for journey in Journey::ALL {
            let cmds = self.commands_in_journey(journey, include_experimental);
            if cmds.is_empty() {
                continue;
            }
            out.push_str(&format!(
                "── {} ── {}\n",
                journey.title(),
                journey.summary()
            ));
            for c in cmds {
                let mark = if c.experimental {
                    " (experimental)"
                } else {
                    ""
                };
                out.push_str(&format!("  {}{}\n    {}\n", c.usage, mark, c.summary));
            }
            out.push('\n');
        }
        out.push_str(
            "machine-readable catalog: registry [--format json]\n\
             per-command flags: <command> --help\n",
        );
        out
    }

    /// Resolve a command name → (group, command). Honors experimental
    /// hiding unless `include_experimental` is true (G-24 gate).
    pub fn find(&self, name: &str, include_experimental: bool) -> Option<(&GroupDef, &CommandDef)> {
        self.commands.iter().find_map(|(g, c)| {
            if c.name == name && (include_experimental || !c.experimental) {
                Some((&self.groups[*g], c))
            } else {
                None
            }
        })
    }

    pub fn groups(&self) -> &[GroupDef] {
        &self.groups
    }

    /// Commands of one group (registration order).
    pub fn commands_in(&self, group: usize, include_experimental: bool) -> Vec<&CommandDef> {
        self.commands
            .iter()
            .filter(|(g, c)| *g == group && (include_experimental || !c.experimental))
            .map(|(_, c)| c)
            .collect()
    }

    /// All registered commands (registration order).
    pub fn commands(&self, include_experimental: bool) -> Vec<&CommandDef> {
        self.commands
            .iter()
            .filter(|(_, c)| include_experimental || !c.experimental)
            .map(|(_, c)| c)
            .collect()
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub fn command_count(&self, include_experimental: bool) -> usize {
        self.commands(include_experimental).len()
    }

    /// Aggregated help: USAGE line + one section per group with each
    /// command's usage + summary (LoopX help surface).
    pub fn render_help(&self, include_experimental: bool) -> String {
        let mut out = String::new();
        out.push_str(
            "FutureOS loop control plane — durable goals, deterministic should-run kernel\n\n",
        );
        out.push_str("USAGE: future-loop <command> [args]\n\n");
        for (idx, group) in self.groups.iter().enumerate() {
            let cmds = self.commands_in(idx, include_experimental);
            if cmds.is_empty() {
                continue;
            }
            out.push_str(&format!("── {} ── {}\n", group.name, group.summary));
            for c in cmds {
                let mark = if c.experimental {
                    " (experimental)"
                } else {
                    ""
                };
                out.push_str(&format!("  {}{}\n    {}\n", c.usage, mark, c.summary));
            }
            out.push('\n');
        }
        out.push_str(&format!(
            "State root: {} (env FUTURE_LOOP_ROOT)",
            std::env::var("FUTURE_LOOP_ROOT").unwrap_or_else(|_| {
                format!(
                    "{}/.future/loop",
                    std::env::var("HOME").unwrap_or_else(|_| ".".into())
                )
            })
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CommandRegistry {
        let mut r = CommandRegistry::new();
        let goal = r.group("goal", "goal lifecycle");
        r.command(
            goal,
            "goal",
            "create a durable goal",
            "goal init --objective \"...\"",
        );
        let ops = r.group("ops", "operations");
        r.command(ops, "backup", "back up a goal", "backup --goal G");
        r.command_experimental(ops, "doctor-x", "experimental probe", "doctor-x --goal G");
        r
    }

    #[test]
    fn groups_and_commands_register_in_order() {
        let r = sample();
        assert_eq!(r.group_count(), 2);
        assert_eq!(r.command_count(false), 2);
        assert_eq!(r.command_count(true), 3);
        let (g, c) = r.find("backup", false).unwrap();
        assert_eq!(g.name, "ops");
        assert_eq!(c.name, "backup");
        // experimental hidden by default
        assert!(r.find("doctor-x", false).is_none());
        assert!(r.find("doctor-x", true).is_some());
        // unknown command
        assert!(r.find("nope", false).is_none());
    }

    #[test]
    fn duplicate_registration_is_idempotent() {
        let mut r = CommandRegistry::new();
        let g = r.group("g", "g");
        r.command(g, "cmd", "one", "cmd");
        r.command(g, "cmd", "two", "cmd");
        assert_eq!(r.command_count(false), 1);
    }

    #[test]
    fn subcommands_register_and_find() {
        let mut r = sample();
        r.subcommand("goal", "init", "create a goal", "init --objective \"...\"");
        r.subcommand("goal", "cancel", "end a goal", "cancel --goal G");
        // idempotent re-register
        r.subcommand("goal", "init", "dup", "init");
        // unknown parent → no-op
        r.subcommand("nope", "x", "y", "z");
        let (_, parent, sub) = r.find_subcommand("goal", "init", false).unwrap();
        assert_eq!(parent.name, "goal");
        assert_eq!(sub.name, "init");
        assert_eq!(sub.summary, "create a goal");
        assert_eq!(parent.subcommands.len(), 2);
        assert!(r.find_subcommand("goal", "cancel", false).is_some());
        assert!(r.find_subcommand("goal", "missing", false).is_none());
        assert!(r.find_subcommand("backup", "init", false).is_none());
    }

    #[test]
    fn journey_assignment_regroups_commands_for_the_operator_view() {
        let mut r = sample();
        // default: everything is maintainer until reassigned
        assert_eq!(r.commands_in_journey(Journey::Maintainer, false).len(), 2);
        r.set_journey("goal", Journey::Starter);
        r.set_journey("backup", Journey::Maintainer);
        // unknown command names are a no-op
        r.set_journey("nope", Journey::Daily);
        let starter = r.commands_in_journey(Journey::Starter, false);
        assert_eq!(starter.len(), 1);
        assert_eq!(starter[0].name, "goal");
        let (g, c) = r.find("goal", false).unwrap();
        assert_eq!(g.name, "goal");
        assert_eq!(c.journey, Journey::Starter);
    }

    #[test]
    fn journeys_render_all_five_sections_in_display_order() {
        let mut r = sample();
        r.set_journey("goal", Journey::Starter);
        r.set_journey("backup", Journey::Daily);
        let text = r.render_journeys(false);
        let start = text.find("── Start here ──").unwrap();
        let daily = text.find("── Daily operator ──").unwrap();
        assert!(start < daily, "display order broken: {text}");
        assert!(text.contains("goal init --objective"), "got: {text}");
        assert!(!text.contains("doctor-x"), "experimental leaked: {text}");
        // empty journeys are skipped; setup/driver/maintainer had no commands
        assert!(!text.contains("── Loop driver ──"), "got: {text}");
        let text_x = r.render_journeys(true);
        assert!(text_x.contains("doctor-x --goal G (experimental)"));
    }

    #[test]
    fn journey_keys_are_stable_for_json() {
        let keys: Vec<&str> = Journey::ALL.iter().map(|j| j.key()).collect();
        assert_eq!(keys, ["starter", "daily", "driver", "setup", "maintainer"]);
    }

    #[test]
    fn help_renders_groups_in_registration_order() {
        let r = sample();
        let help = r.render_help(false);
        assert!(help.contains("── goal ──"));
        assert!(help.contains("── ops ──"));
        assert!(help.contains("backup --goal G"));
        assert!(!help.contains("doctor-x"));
        let help_x = r.render_help(true);
        assert!(help_x.contains("doctor-x --goal G (experimental)"));
    }
}
