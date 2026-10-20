//! One-time, pre-runtime host setup. Helper dispatch never enters this path.
static INITIALIZED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();

pub(crate) fn initialize() -> Result<(), crate::AppError> {
    INITIALIZED
        .get_or_init(|| {
            future_host::environment::prepare_process();
            // Before the first store access, because an invitation minted by
            // this launch has to know which relay it points at — and that choice
            // dies with the process, so nothing is read back from disk here.
            crate::launch::apply_relay_from_argv();
            crate::cli_tool::prepare_path().map_err(|e| e.to_string())
        })
        .clone()
        .map_err(Into::into)
}

/// Separate host log prevents attachment/restart from opening an external
/// Agent's log. Call after acquiring the Desktop lock; it lives until exit.
pub(crate) fn start_logging() -> Result<(), crate::AppError> {
    future_host::environment::start_logging()?;
    if let Err(error) = crate::cli_tool::resolve() {
        report(&error.to_string());
    }
    Ok(())
}

#[cfg(all(test, feature = "gui"))]
pub(crate) use future_host::environment::log_file_path;
pub(crate) use future_host::environment::report;

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with no resolvable CLI still starts logging: the missing tool is
    /// *reported* through the log and the version line is written either way, so
    /// an uninstalled CLI never fails the launch. Whether `cli_tool::resolve()`
    /// succeeds is what selects the arm, so the fixture pins it instead of
    /// leaving this line at the mercy of the host's PATH.
    #[test]
    fn a_host_without_a_resolvable_cli_still_starts_logging() {
        #[cfg(all(windows, feature = "gui"))]
        if crate::test_process::reexec(
            "host_environment::tests::a_host_without_a_resolvable_cli_still_starts_logging",
        ) {
            return;
        }
        let _home = crate::auth_store::test_support::HomeGuard::new("host-start-logging");
        let empty = tempfile::tempdir().expect("an empty directory to search");
        // A PATH to restore is a precondition of this fixture, not a branch: the
        // `None` arm would be a line no run executes.
        let saved = std::env::var_os("PATH").expect("this host has a PATH to restore");
        // The test rebuild's candidates are the executable's own directory and
        // PATH; neither holds a `future` here (the dev fallbacks are
        // `#[cfg(not(test))]`).
        std::env::set_var("PATH", empty.path());
        let unresolvable = crate::cli_tool::resolve().is_err();
        let result = start_logging();
        std::env::set_var("PATH", saved);
        assert!(
            unresolvable,
            "the fixture needs a host where `future` cannot resolve"
        );
        assert!(result.is_ok(), "{result:?}");
    }
}
