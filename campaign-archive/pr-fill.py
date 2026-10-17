#!/usr/bin/env python3
"""Populate the PR worktree with exactly the campaign's real changes.

Run inside .worktrees/cov100-pr (a checkout of origin/main on test/cov100-coverage).
Takes every path the campaign branch changed under a PRODUCT tree from that branch,
so the PR carries my changes and leaves upstream's own newer work untouched.

Why not `git diff | git apply`: the campaign branch also tracks 597 files under
`.future/` (gitignored upstream; 11.1M of the 12.0M "insertions" are run journals).
Checking out the allowlisted paths sidesteps them entirely.
"""
import subprocess
import sys

CAMPAIGN = "test/cov100-r2"
TOPS = ["agent", "channels", "cli", "desktop", "docs", "mobile",
        "orchestration", "packages", "scripts", "tui"]
FILES = ["AGENTS.md", "CLAUDE.md", "Cargo.toml", "Cargo.lock", "Makefile",
         "README.md", "rust-toolchain.toml", "package.json", "package-lock.json",
         "tsconfig.json", ".gitignore", ".github"]


def git(*a, check=True):
    p = subprocess.run(["git", *a], capture_output=True, text=True)
    if check and p.returncode != 0:
        print(p.stderr, file=sys.stderr)
        sys.exit(1)
    return p.stdout


def main():
    changed = git("diff", "--name-status", f"origin/main..{CAMPAIGN}",
                  "--", *TOPS, *FILES).splitlines()
    add, deleted = [], []
    for line in changed:
        st, path = line.split("\t", 1)
        (deleted if st.startswith("D") else add).append(path)
    print(f"campaign changed: {len(add)} present + {len(deleted)} deleted")

    # batch by chunks so the command line stays sane
    for i in range(0, len(add), 120):
        chunk = add[i:i + 120]
        git("checkout", CAMPAIGN, "--", *chunk)
    for path in deleted:
        git("rm", "-q", "--ignore-unmatch", path, check=False)

    st = git("status", "--short")
    n = len([l for l in st.splitlines() if l.strip()])
    print(f"staged: {n} files")
    # honest check: nothing under .future may appear
    bad = [l for l in st.splitlines() if ".future/" in l]
    print("payload contains .future/ entries:", len(bad))
    return 0


if __name__ == "__main__":
    sys.exit(main())
