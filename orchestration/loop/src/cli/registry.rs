//! CLI command registration framework (G-26) — LoopX `cli.py` + the
//! `register_*_commands()` pattern, natively.
//!
//! The registry is the ecosystem injection point: a command group is
//! declared once, commands register into it with a one-line summary and a
//! usage line, and `--help` aggregates everything by group instead of a
//! hard-coded help string. Consumers register through the same API:
//!
//! - capability command hooks (G-24) register per-capability commands
//!   (hidden for `experimental` capabilities unless `--include-experimental`);
//! - extension manifests (G-21) register declared commands as provider
//!   metadata (v1 is declarative — no native code dispatch, so those commands
//!   surface in help/catalog but execution is a P4 runtime concern).
//!
//! Dispatch stays in the binary: the registry validates `args[0]` against the
//! registered command set (unknown command → error with a hint) and the
//! binary routes to its handler. Handlers keep their heterogeneous signatures
//! (`&Store` / `&mut Store` / async) — the registry does not force a uniform
//! handler type, which keeps the migration a pure assembly change.

/// One registered command.
#[derive(Debug, Clone)]
pub struct CommandDef {
    pub name: String,
    pub summary: String,
    pub usage: String,
    /// True for commands surfaced only with `--include-experimental`
    /// (capability hooks from `experimental` capabilities).
    pub experimental: bool,
}

/// One registered command group (LoopX command groups: goal / todo / agent /
/// capability / extension / ops / multi-agent / work-items / cli).
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
        // Idempotent: re-registering the same name in the same group is a no-op
        // (extension install may re-declare a manifest that provides commands).
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
                },
            ));
        }
        self
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
