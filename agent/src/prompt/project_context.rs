//! Cwd-scoped project instructions, shared by startup, reload and run prompts.

#[derive(Default)]
pub(crate) struct ProjectContext {
    pub content: String,
    file_name: Option<&'static str>,
}

impl ProjectContext {
    pub fn file_names(&self) -> Vec<String> {
        self.file_name.into_iter().map(str::to_owned).collect()
    }
}

/// Select the first readable file, including empty files. Do not merge files
/// or walk parent directories. Disabling discovery does not disable FUTURE.md.
pub(crate) fn load_project_context(cwd: &str, enabled: bool) -> ProjectContext {
    if enabled {
        for file_name in ["AGENTS.md", "CLAUDE.md", "GEMINI.md"] {
            if let Ok(content) = std::fs::read_to_string(std::path::Path::new(cwd).join(file_name))
            {
                return ProjectContext {
                    content,
                    file_name: Some(file_name),
                };
            }
        }
    }
    ProjectContext::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_fallback_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        for name in ["GEMINI.md", "CLAUDE.md", "AGENTS.md"] {
            std::fs::write(dir.path().join(name), name).unwrap();
            let context = load_project_context(cwd, true);
            assert_eq!(context.content, name);
            assert_eq!(context.file_names(), [name]);
        }
        std::fs::write(dir.path().join("AGENTS.md"), "").unwrap();
        let context = load_project_context(cwd, true);
        assert!(context.content.is_empty());
        assert_eq!(context.file_names(), ["AGENTS.md"]);

        std::fs::remove_file(dir.path().join("AGENTS.md")).unwrap();
        std::fs::create_dir(dir.path().join("AGENTS.md")).unwrap();
        assert_eq!(load_project_context(cwd, true).file_names(), ["CLAUDE.md"]);
        std::fs::write(dir.path().join("CLAUDE.md"), [0xff]).unwrap();
        assert_eq!(load_project_context(cwd, true).file_names(), ["GEMINI.md"]);
    }

    #[test]
    fn discovery_is_cwd_scoped_and_can_be_disabled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "parent rules").unwrap();
        let child = dir.path().join("child");
        std::fs::create_dir(&child).unwrap();
        for context in [
            load_project_context(dir.path().to_str().unwrap(), false),
            load_project_context(child.to_str().unwrap(), true),
        ] {
            assert!(context.content.is_empty());
            assert!(context.file_names().is_empty());
        }
    }
}
