//! Two ledger/runtime write failures reached from commands whose FIRST write is the
//! one that fails - which is what makes them reachable at all, unlike a mid-run
//! append (earlier writes would fail first).
//!
//! Both failures are induced portably: a read-only file works on Windows and on
//! Unix, and a directory standing in for a file works everywhere.

mod common;

use common::{cli, cli_err, cli_ok, cli_root, init_goal, open_store};

/// A workbench markdown that yields at least one event, so the append loop runs.
const MD: &str = "---\nstatus: active\n---\n\n# Active Goal State\n\n\
## Agent Todo\n\n\
- [ ] [P0] Imported task\n  <!-- future-loop:todo todo_id=todo_bf1 status=open action_kind=shell -->\n";

/// `cmd_backfill`'s ONLY ledger write is the append loop, so a ledger that can be
/// READ but not APPENDED makes that `?` the first failure - the command must report it
/// rather than print a success line for events it never recorded.
///
/// This is the reason the earlier read-only-ledger test could not reach this arm: a
/// `run` appends during claim/boundary work long before the interesting sites, so the
/// failure surfaces earlier. `backfill` reads the goal (a plain read) and then writes.
#[test]
fn backfill_reports_a_ledger_it_cannot_append_to() {
    let cr = cli_root();
    let goal = init_goal(&cr, "backfill into a read-only ledger");
    let md = std::path::Path::new(&cr.root).join("workbench.md");
    std::fs::write(&md, MD).unwrap();

    let ledger = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("events.jsonl");
    let original = std::fs::metadata(&ledger).unwrap().permissions();
    let mut locked = original.clone();
    locked.set_readonly(true);
    std::fs::set_permissions(&ledger, locked).expect("the ledger must become read-only");

    let outcome = cli(&["backfill", "--goal", &goal, "--from", &md.to_string_lossy()]);
    // Restore the saved mode (not a forced writable one: `set_readonly(false)` is a
    // security lint and would discard the original mode).
    let _ = std::fs::set_permissions(&ledger, original.clone());

    let err = outcome.expect_err("an unappendable ledger must fail the command");
    assert!(
        !err.contains("goal") || !err.contains("not found"),
        "the failure must be the WRITE, not a missing goal: {err}"
    );

    // Nothing was recorded, so a caller must not believe the import happened.
    let store = open_store(&cr);
    let state = store.replay(&goal).unwrap().unwrap();
    let imported = state.todos.iter().filter(|t| t.id == "todo_bf1").count();
    assert_eq!(
        imported,
        0,
        "a refused append must leave no imported todo behind: {:?}",
        state.todos.iter().map(|t| &t.id).collect::<Vec<_>>()
    );

    // And the diagnostic path is the one that owns the write: a `--dry-run` of the
    // SAME workbench succeeds (it never appends), which proves the fixture's markdown
    // is valid and the failure above was the append, not the parse.
    let dry = cli(&[
        "backfill",
        "--goal",
        &goal,
        "--from",
        &md.to_string_lossy(),
        "--dry-run",
    ]);
    assert!(
        dry.is_ok(),
        "the markdown must parse on the dry-run path, or the test proves nothing \
         about the append: {dry:?}"
    );
}

/// `cmd_runs compact --cutoff` archives runs by MOVING their artifacts into
/// `<goal>/runs/archive/`. With that path occupied by a regular file the move cannot
/// happen, and the command must report the failure instead of claiming a compaction
/// that never occurred (a caller would otherwise trust a report listing archived runs).
#[test]
fn compact_cutoff_reports_an_archive_it_cannot_create() {
    let cr = cli_root();
    let goal = init_goal(&cr, "compact with a blocked archive");
    // Seed run artifacts the way the runtime indexes them: `compat::write_run`
    // writes into the goal's runs dir and `rebuild_index` derives the index from
    // those files (mirrors `misc_drive`'s working fixture). `Store::append_run` alone
    // does NOT put a file where the compaction scan looks, which is why an earlier
    // version of this test saw zero rows and silently archived nothing.
    let store = open_store(&cr);
    for (idx, todo) in [(1u32, "todo_c1"), (2u32, "todo_c2")] {
        future_loop::compat::write_run(
            &store.goal_dir(&goal),
            &goal,
            &common::run_record(todo, "completed", idx as u64),
        )
        .unwrap();
    }
    drop(store);
    let indexed = future_loop::runtime::run_index::rebuild_index(&cr.root, &goal).unwrap();
    assert!(
        indexed.rows_written >= 1,
        "the fixture must produce indexed rows, or the compaction has nothing to move"
    );

    // Occupy the archive path with a FILE: `create_dir_all` then fails, so the move
    // that the report is about cannot succeed.
    let archive = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("runs")
        .join("archive");
    std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
    if archive.exists() {
        std::fs::remove_dir_all(&archive).ok();
    }
    std::fs::write(&archive, b"not a directory").unwrap();

    let err = cli_err(&["runs", "compact", "--goal", &goal, "--cutoff", "9999999999"]);
    assert!(
        !err.contains("--cutoff must be epoch secs"),
        "the cutoff must be a valid epoch, or the arm under test is not the archive \
         move: {err}"
    );
    // The archive is still a file: the command failed rather than silently pretending.
    assert!(
        archive.is_file(),
        "the fixture must still block the archive path"
    );
}

/// The counterpart: with a usable archive path the SAME command succeeds and reports
/// the runs it archived, so the failure above is attributable to the blocked path and
/// not to the fixture being unarchivable in general.
#[test]
fn compact_cutoff_succeeds_when_the_archive_path_is_usable() {
    let cr = cli_root();
    let goal = init_goal(&cr, "compact with a usable archive");
    let store = open_store(&cr);
    for (idx, todo) in [(1u32, "todo_d1"), (2u32, "todo_d2")] {
        future_loop::compat::write_run(
            &store.goal_dir(&goal),
            &goal,
            &common::run_record(todo, "completed", idx as u64),
        )
        .unwrap();
    }
    drop(store);
    future_loop::runtime::run_index::rebuild_index(&cr.root, &goal).unwrap();

    cli_ok(&["runs", "compact", "--goal", &goal, "--cutoff", "9999999999"]);

    // The runs moved into the archive, so the index now points there - and they are
    // still indexed (archival is recoverable, never a delete).
    let rebuilt = future_loop::runtime::run_index::rebuild_index(&cr.root, &goal).unwrap();
    assert!(
        rebuilt.rows_written > 0,
        "the archived runs must still be indexed (recoverable, never deleted)"
    );
    let archive = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("runs")
        .join("archive");
    assert!(
        archive.is_dir(),
        "the archive directory must exist after a successful compaction"
    );
}
