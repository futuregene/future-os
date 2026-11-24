//! The process's file-descriptor budget, and what a store that fails to open
//! has to say about it.
//!
//! Shared by the halves of a running installation that hit the same ceiling: the
//! Desktop host (`futureos`, and `future host`) opens the app store, the tasks
//! store and — when the Agent runs in-process — every conversation's SQLite
//! handles, while the Agent's event journal opens the next handle on the same
//! budget. A launch started by Finder or launchd inherits a soft
//! `RLIMIT_NOFILE` of **256** on macOS, so that ceiling is shared by all of
//! them: measured 2026-11-25 on the running Desktop (pid 90103, 14 h uptime),
//! 185 descriptors in steady state — 12 consecutive one-second samples, all
//! 185 — of which 44 are SQLite (`agent.db` 22, `app.db` 13, `tasks.db` 5).
//! That is 72 % of the budget before any burst of workers, where each session
//! opens a database, its `-wal` and its `-shm`.
//!
//! Running out of descriptors does not look like running out of descriptors.
//! SQLite reports `SQLITE_CANTOPEN` (`code=14`, "unable to open database file")
//! for a failed `open`, and the same code covers an `open` that hit `ENOSPC` —
//! so a full disk and a descriptor ceiling are indistinguishable in the log.
//! Measured 2026-11-24 10:08:52Z: `SQLITE_FULL` (13) and `CANTOPEN` (14) in the
//! same window, which first read as an fd problem and later as a full disk.
//! Hence the two halves of this crate: the raise ([`ensure_fd_headroom`]) and
//! the fields that tell the two apart on the next failure
//! ([`open_failure_detail`]).
//!
//! The shape of [`ensure_fd_headroom`] is the TUI's `ensure_fd_headroom`
//! (`tui/src/lib.rs:141`) — the production fix for the same cliff in
//! `future tui`. It is not imported from there (the TUI deliberately links no
//! application backend, and the Agent's own copy at `agent/src/lib.rs:129` is
//! `#[cfg(test)]`), so the production raise lives here, once, and a host entry
//! point reaches it through `future_host::environment::prepare_process_for`
//! before the first store is opened.

/// The soft limit a host process asks for.
///
/// Sized like the Agent suite's `WANT` (`agent/src/lib.rs`, 8192): it is ~4× the
/// highest descriptor count measured in that binary and far above the 256 a
/// Finder launch inherits, while staying well inside what an ordinary shell's
/// hard limit already allows. The raise is capped by the hard limit, so a
/// smaller hard limit simply means a smaller budget.
pub const WANT: u64 = 8192;

/// Largest descriptor number the probe in [`open_fds`] will test.
///
/// A normal host runs with a soft limit of a few thousand; the cap only matters
/// on a machine whose soft limit is a million, where probing the whole table
/// would cost more than the line is worth.
const MAX_PROBED_FDS: u64 = 65_536;

/// The kernel's `RLIMIT_NOFILE` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub soft: u64,
    pub hard: u64,
}

/// What [`ensure_fd_headroom`] found and left behind, for one log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    /// The limits before the raise; `None` when the platform has no such limit.
    pub before: Option<Limits>,
    /// The limits read back afterwards — the kernel's answer, not the request.
    pub after: Option<Limits>,
    /// Why the raise did not take effect, when it did not.
    pub error: Option<String>,
}

impl Budget {
    fn unsupported() -> Self {
        Self {
            before: None,
            after: None,
            error: None,
        }
    }

    /// One line naming the budget, whether it moved, and the descriptors in use.
    pub fn line(&self) -> String {
        let open = match open_fds() {
            Some(open) => open.to_string(),
            None => "n/a".into(),
        };
        let (Some(before), Some(after)) = (self.before, self.after) else {
            return format!("fd budget: no RLIMIT_NOFILE on this platform (open={open})");
        };
        match &self.error {
            Some(error) => format!(
                "fd budget: raise to {} failed, still {}/{} (open={open}): {error}",
                WANT.min(before.hard),
                after.soft,
                after.hard
            ),
            None if after.soft > before.soft => format!(
                "fd budget: soft {} -> {} (hard {}, open={open})",
                before.soft, after.soft, after.hard
            ),
            None => format!(
                "fd budget: soft {}/{} unchanged (open={open})",
                after.soft, after.hard
            ),
        }
    }
}

/// Raise the soft descriptor limit to `min(WANT, hard)`.
///
/// Upward only, idempotent and never fatal: a process already at or above the
/// target is left exactly as it is, a hard limit below [`WANT`] caps the target
/// instead of being exceeded, and a raise the kernel refuses (a restricted
/// launch, a container) leaves the process running under the budget it had.
/// The caller logs the outcome; nothing here panics or prints.
#[cfg(unix)]
pub fn ensure_fd_headroom() -> Budget {
    let Some(before) = limits() else {
        return Budget::unsupported();
    };
    let target = WANT.min(before.hard);
    if before.soft >= target {
        return Budget {
            before: Some(before),
            after: Some(before),
            error: None,
        };
    }
    let error = set_soft_limit(target, before.hard)
        .err()
        .map(|error| error.to_string());
    Budget {
        before: Some(before),
        // Re-read instead of trusting the request: what the kernel granted is
        // the only thing worth logging.
        after: limits(),
        error,
    }
}

/// Windows has no `RLIMIT_NOFILE`: a process is bounded by its handle table,
/// which no single number describes and no call raises.
#[cfg(not(unix))]
pub fn ensure_fd_headroom() -> Budget {
    Budget::unsupported()
}

/// The kernel's `RLIMIT_NOFILE` soft/hard pair, when the platform has one.
#[cfg(unix)]
pub fn limits() -> Option<Limits> {
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` only writes the struct it is given for `RLIMIT_NOFILE`;
    // a failure is reported by the return value and means "no answer here".
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } != 0 {
        return None;
    }
    // `rlim_t` is a 64-bit type on every host this runs on, so this is the
    // limit as the kernel holds it.
    Some(Limits {
        soft: limits.rlim_cur,
        hard: limits.rlim_max,
    })
}

#[cfg(not(unix))]
pub fn limits() -> Option<Limits> {
    None
}

/// How many descriptors this process holds right now.
///
/// A probe loop rather than `/proc/self/fd` (which macOS does not have):
/// `fcntl(F_GETFD)` answers `EBADF` for a closed slot, so the count is exact and
/// portable across the Unix hosts. Bounded by the soft limit — 8192 probes cost
/// about a millisecond — and read only at startup and on a failed store open,
/// never per request.
#[cfg(unix)]
pub fn open_fds() -> Option<usize> {
    let ceiling = limits()
        .map(|limits| limits.soft)
        .unwrap_or(256)
        .min(MAX_PROBED_FDS);
    let mut open = 0usize;
    for fd in 0..ceiling as libc::c_int {
        // SAFETY: `fcntl` with `F_GETFD` only reads that descriptor's flags.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1 {
            open += 1;
        }
    }
    Some(open)
}

#[cfg(not(unix))]
pub fn open_fds() -> Option<usize> {
    None
}

/// The fields a failed store open must carry so the next such line is not read
/// as "the database is broken".
///
/// `13` is `SQLITE_FULL` (a full disk); `14` is `SQLITE_CANTOPEN` ("cannot open
/// the file"), which is also what SQLite reports when `open` hits `ENOSPC`. The
/// descriptor count and the limit pair beside the code are what separate a
/// descriptor ceiling from a full disk at a glance.
pub fn open_failure_detail(code: Option<i32>) -> String {
    let limit = match limits() {
        Some(limits) => format!("{}/{}", limits.soft, limits.hard),
        None => "n/a".into(),
    };
    let fds = match open_fds() {
        Some(open) => open.to_string(),
        None => "n/a".into(),
    };
    let code = match code {
        Some(code) => code.to_string(),
        None => "none".into(),
    };
    format!(
        "fds={fds} limit={limit} code={code} (13=disk full, 14=cannot open; \
         an open that hits a full disk also reports 14)"
    )
}

/// The SQLite primary result code of a `rusqlite` error.
///
/// Only the low byte is used: SQLite's extended codes keep the primary code
/// there, so `SQLITE_CANTOPEN_ISDIR` still reads as `14`.
pub fn sqlite_code(error: &rusqlite::Error) -> Option<i32> {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => Some(failure.extended_code & 0xff),
        _ => None,
    }
}

/// The SQLite primary result code anywhere in an error chain.
///
/// For callers whose error is a wrapper (`anyhow`, or an error enum) rather
/// than a `rusqlite` one. Each node is downcast, so a wrapper that forwards
/// `source()` past its own payload — `#[error(transparent)]` and
/// `std::io::Error::new` both do — still needs to hand in the payload itself
/// ([`sqlite_code`]) rather than the wrapper.
pub fn sqlite_code_in_chain(error: &(dyn std::error::Error + 'static)) -> Option<i32> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = current {
        if let Some(error) = error.downcast_ref::<rusqlite::Error>() {
            return sqlite_code(error);
        }
        current = error.source();
    }
    None
}

#[cfg(unix)]
fn set_soft_limit(soft: u64, hard: u64) -> std::io::Result<()> {
    let limits = libc::rlimit {
        rlim_cur: soft as libc::rlim_t,
        rlim_max: hard as libc::rlim_t,
    };
    // SAFETY: `setrlimit` only reads the struct it is given, and this call raises
    // our own soft limit towards a hard limit the process already holds.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Run a descriptor-limit test in a child process of its own.
    ///
    /// The limits are process-global and a lowered hard limit cannot be raised
    /// again, so the body must never run beside other tests in the same process.
    /// Returns `true` in the parent (the child's exit status is the assertion)
    /// and `false` in the child, whose body then runs.
    fn reexec_test() -> bool {
        let thread = std::thread::current();
        let name = thread.name().expect("named libtest thread");
        const CHILD: &str = "FUTURE_FD_ISOLATED_LIMIT_TEST";
        if std::env::var(CHILD).as_deref() == Ok(name) {
            return false;
        }
        let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", name, "--nocapture", "--test-threads=1"])
            .env(CHILD, name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn isolated descriptor-limit test");
        let output = child.wait_with_output().expect("isolated test result");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout.contains("running 1 test")
                && stdout.contains("1 passed; 0 failed"),
            "isolated {name} failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        true
    }

    /// Lower this process's soft limit to `soft`, leaving the hard limit alone.
    fn lower_soft_to(soft: u64) {
        let hard = limits().expect("a unix host has RLIMIT_NOFILE").hard;
        set_soft_limit(soft, hard).expect("lowering the soft limit is allowed");
        assert_eq!(limits().unwrap().soft, soft);
    }

    /// A Desktop launched from Finder starts at 256. The raise has to reach the
    /// budget (or the hard ceiling) from there — this is the case the fix
    /// exists for, so it runs in a child process where lowering the limit is
    /// harmless.
    #[test]
    fn a_lowered_soft_limit_is_raised_to_the_budget() {
        if reexec_test() {
            return;
        }
        let hard = limits().expect("a unix host has RLIMIT_NOFILE").hard;
        assert!(
            hard > 256,
            "this test needs a hard limit above 256 to prove a raise, got {hard}"
        );
        lower_soft_to(256);

        let budget = ensure_fd_headroom();

        let after = limits().unwrap();
        assert_eq!(after.soft, WANT.min(hard), "budget line: {}", budget.line());
        assert_eq!(after.hard, hard, "the hard limit is never touched");
        assert_eq!(budget.before.map(|l| l.soft), Some(256));
        assert!(budget.error.is_none(), "{}", budget.line());
    }

    /// Calling it again is a no-op: never lower an already-raised limit, never
    /// ask the kernel for more than the hard limit allows.
    #[test]
    fn a_second_call_neither_lowers_nor_exceeds_the_hard_limit() {
        if reexec_test() {
            return;
        }
        let hard = limits().expect("a unix host has RLIMIT_NOFILE").hard;
        assert!(hard > 256, "this test needs a hard limit above 256");
        // Start at the state the raise leaves behind.
        let target = WANT.min(hard);
        set_soft_limit(target, hard).expect("set the raised state");
        assert_eq!(limits().unwrap().soft, target);

        ensure_fd_headroom();
        assert_eq!(
            limits().unwrap().soft,
            target,
            "a second call must not move it"
        );
        let budget = ensure_fd_headroom();
        let after = limits().unwrap();
        assert_eq!(after.soft, target);
        assert!(after.soft <= after.hard, "{}", budget.line());
        assert!(budget.error.is_none(), "{}", budget.line());
    }

    /// A hard limit below the budget caps the target instead of being exceeded
    /// or forced: the process keeps a smaller budget and keeps running.
    #[test]
    fn a_hard_limit_below_the_budget_is_respected() {
        if reexec_test() {
            return;
        }
        // Lowering the hard limit is irreversible, which is exactly why this
        // runs in a child process of its own.
        set_soft_limit(256, 256).expect("lower hard and soft together");
        assert_eq!(
            limits().unwrap(),
            Limits {
                soft: 256,
                hard: 256
            }
        );

        let budget = ensure_fd_headroom();

        assert_eq!(
            limits().unwrap(),
            Limits {
                soft: 256,
                hard: 256
            }
        );
        assert!(
            budget.error.is_none(),
            "no raise was needed: {}",
            budget.line()
        );
        assert!(
            budget.line().contains("unchanged"),
            "the line must say the budget stayed put: {}",
            budget.line()
        );
    }

    /// The detail attached to a failed store open answers "descriptor ceiling
    /// or full disk?" without a second investigation.
    #[test]
    fn the_open_failure_detail_carries_the_budget_and_the_code_meaning() {
        let detail = open_failure_detail(Some(14));
        assert!(detail.contains("fds="), "{detail}");
        assert!(detail.contains("limit="), "{detail}");
        assert!(detail.contains("code=14"), "{detail}");
        assert!(detail.contains("13=disk full"), "{detail}");
        assert!(detail.contains("14=cannot open"), "{detail}");
        // No SQLite code is still a complete line: the fields never vanish.
        let detail = open_failure_detail(None);
        assert!(detail.contains("code=none"), "{detail}");
        assert!(detail.contains("limit="), "{detail}");
    }

    /// A caller's error type: the shape a store error has in production
    /// (`#[error(transparent)] Database(#[from] rusqlite::Error)`), reduced to
    /// the one property this helper reads.
    #[derive(Debug)]
    struct Wrapper(Option<rusqlite::Error>);

    impl std::fmt::Display for Wrapper {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "storage failure")
        }
    }

    impl std::error::Error for Wrapper {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.0
                .as_ref()
                .map(|error| error as &(dyn std::error::Error + 'static))
        }
    }

    /// `SQLITE_CANTOPEN` keeps its primary code even when SQLite extends it
    /// (`_ISDIR`, `_FULLPATH`, …), which is what the log line compares — and the
    /// code is found through the wrapper the caller actually holds.
    #[test]
    fn an_extended_sqlite_code_reports_its_primary_code_through_a_wrapper() {
        let sqlite = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::CannotOpen,
                extended_code: 526, // SQLITE_CANTOPEN_ISDIR
            },
            None,
        );
        assert_eq!(sqlite_code(&sqlite), Some(14));
        assert_eq!(
            sqlite_code_in_chain(&Wrapper(Some(sqlite))),
            Some(14),
            "the code is found through a wrapper that carries it as its cause"
        );
        assert_eq!(
            sqlite_code_in_chain(&Wrapper(None)),
            None,
            "an error without a SQLite cause reports no code"
        );
    }
}

#[cfg(all(test, not(unix)))]
mod windows_tests {
    use super::*;

    /// Without `RLIMIT_NOFILE` there is nothing to raise, and the line a failure
    /// carries still names the fields it cannot fill.
    #[test]
    fn the_budget_is_reported_as_not_applicable() {
        assert!(limits().is_none());
        assert!(open_fds().is_none());
        let budget = ensure_fd_headroom();
        assert!(
            budget.line().contains("no RLIMIT_NOFILE"),
            "{}",
            budget.line()
        );
        let detail = open_failure_detail(Some(14));
        assert!(detail.contains("fds=n/a"), "{detail}");
        assert!(detail.contains("limit=n/a"), "{detail}");
        assert!(detail.contains("code=14"), "{detail}");
    }
}
