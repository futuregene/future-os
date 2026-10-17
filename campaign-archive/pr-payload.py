#!/usr/bin/env python3
"""Build the PR payload: from this worktree's HEAD, stage ONLY the real changes.

WHY. `git diff origin/main..HEAD` reports 1026 files and 12.0M insertions, but 414 of
those files are `.future/**` - a directory `.gitignore` excludes (lines 42 and 90) and
which `origin/main` does not track at all. They are in this branch because I once ran
`git add -f .future/` and because `snapshot.sh` commits with `-A`-like semantics. They
must not enter a PR: 11.1M of the 12.0M lines are `.future/goal/runs/*.jsonl` (the agent's
run journals), plus multi-hundred-thousand-line lcov captures.

So a PR cannot be "this branch". The PR is built as: start from origin/main, then take the
working tree exactly as it is here, but only for paths the repository actually tracks.

usage:
  pr-payload.py --list              # show what would be included / excluded
  pr-payload.py --branch NAME       # create NAME from origin/main with the payload

PREREQUISITE: merge `origin/main` into the campaign branch first and commit. The campaign
branch went 13 commits behind during the work, so `git diff origin/main..HEAD` ALSO shows
origin/main's own newer work as if it were a reversion of mine (that is what the six
`.github/` entries were). After the merge, each included path holds the merged content, so
checking it out onto a fresh origin/main base carries my changes and leaves main's alone.
"""
import argparse
import subprocess
import sys

# Top-level trees that hold real product work. Anything else is either campaign
# bookkeeping (.future), editor/OS noise, or untracked scratch.
INCLUDE_TOPS = {
    "agent", "channels", "cli", "desktop", "docs", "mobile",
    "orchestration", "packages", "tui", "scripts",
}
INCLUDE_FILES = {"AGENTS.md", "CLAUDE.md", "Cargo.toml", "Cargo.lock", "Makefile",
                 "README.md", "rust-toolchain.toml", "package.json",
                 "package-lock.json", "tsconfig.json", ".gitignore"}


def git(*args, check=True):
    p = subprocess.run(["git", *args], capture_output=True, text=True)
    if check and p.returncode != 0:
        sys.exit(f"git {' '.join(args)} failed:\n{p.stderr}")
    return p.stdout


def classify():
    """-> (include, excluded_by_tree) from the branch's diff against origin/main."""
    raw = git("diff", "--name-only", "origin/main..HEAD").splitlines()
    inc, exc = [], {}
    for path in raw:
        top = path.split("/", 1)[0]
        if top in INCLUDE_TOPS or path in INCLUDE_FILES:
            inc.append(path)
        else:
            exc[top] = exc.get(top, 0) + 1
    return inc, exc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--branch")
    a = ap.parse_args()

    inc, exc = classify()
    print(f"include: {len(inc)} files under {', '.join(sorted(INCLUDE_TOPS))}")
    print(f"exclude: {sum(exc.values())} files ("
          + ", ".join(f"{k}/ {v}" for k, v in sorted(exc.items(), key=lambda kv: -kv[1]))
          + ")")
    if a.list or not a.branch:
        return 0

    # 1. a clean branch at origin/main
    git("checkout", "-B", a.branch, "origin/main")
    # 2. take the payload's content from the campaign branch, path by path
    for path in inc:
        exists = subprocess.run(["git", "cat-file", "-e", f"test/cov100-r2:{path}"],
                                capture_output=True).returncode == 0
        if exists:
            git("checkout", "test/cov100-r2", "--", path)
        else:
            # deleted on the campaign branch: mirror the deletion
            git("rm", "-q", "--ignore-unmatch", path, check=False)
    git("add", "-A", *sorted(INCLUDE_TOPS), check=False)
    st = git("status", "--short")
    print("\nstaged files:", len([l for l in st.splitlines() if l.strip()]))
    print(st[:2000])
    return 0


if __name__ == "__main__":
    sys.exit(main())
