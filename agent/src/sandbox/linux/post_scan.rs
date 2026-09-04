//! Bounded end-of-command presence checks, not an access audit or enforcement.
use std::path::{Path, PathBuf};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PresenceReport {
    pub present: usize,
    pub failed: usize,
    pub unchecked: usize,
}

pub(crate) fn scan(
    paths: &[PathBuf],
    stop: &dyn Fn() -> bool,
    mut inspect: impl FnMut(&Path) -> std::io::Result<()>,
) -> PresenceReport {
    let mut report = PresenceReport::default();
    for (index, path) in paths.iter().enumerate() {
        if stop() {
            report.unchecked = paths.len() - index;
            break;
        }
        match inspect(path) {
            Ok(()) => report.present += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => report.failed += 1,
        }
        // A filesystem call cannot be preempted by this cooperative budget.
        // Check again before proceeding to the next path.
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_do_not_hide_later_present_paths() {
        let paths = ["/denied", "/bad-parent", "/present", "/absent"].map(PathBuf::from);
        let report = scan(&paths, &|| false, |path| match path.to_str().unwrap() {
            "/denied" => Err(std::io::ErrorKind::PermissionDenied.into()),
            "/bad-parent" => Err(std::io::ErrorKind::NotADirectory.into()),
            "/present" => Ok(()),
            _ => Err(std::io::ErrorKind::NotFound.into()),
        });
        assert_eq!(
            report,
            PresenceReport {
                present: 1,
                failed: 2,
                unchecked: 0
            }
        );
    }

    #[test]
    fn cancellation_or_deadline_records_unchecked_targets() {
        let paths = [PathBuf::from("/one"), PathBuf::from("/two")];
        let inspected = std::cell::Cell::new(0);
        let report = scan(&paths, &|| inspected.get() == 1, |_| {
            inspected.set(inspected.get() + 1);
            Ok(())
        });
        assert_eq!(
            report,
            PresenceReport {
                present: 1,
                failed: 0,
                unchecked: 1
            }
        );
        let report = scan(&paths, &|| true, |_| panic!("must not inspect"));
        assert_eq!(report.unchecked, 2);
    }

    #[test]
    fn create_then_delete_is_an_explicit_snapshot_limitation() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("transient");
        std::fs::write(&target, "data").unwrap();
        std::fs::remove_file(&target).unwrap();
        let report = scan(&[target], &|| false, |p| {
            std::fs::symlink_metadata(p).map(|_| ())
        });
        assert_eq!(report, PresenceReport::default());
    }
}
