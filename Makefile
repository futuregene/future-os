.PHONY: help version \
	build build-cli build-cli-local build-desktop build-desktop-local build-local build-desktop-headless desktop-frontend build-mobile-android build-mobile-ios desktop-sidecars \
	test test-agent test-channels test-cli test-app-settings test-tui test-cli-diff test-tui-diff test-tui-tmux \
	test-desktop test-desktop-rust test-mobile \
	lint lint-rust lint-desktop stylelint-desktop lint-mobile check-desktop check-mobile fmt \
	check-docs test-docs-check \
	run-agent run-tui run-cli run-host run-headless run-desktop run-mobile-android run-mobile-ios run-channels run-goal \
	profile-agent-build profile-agent profile-quick profile-heap \
	generate-models generate-proto \
	install install-cli install-local install-desktop install-desktop-headless install-skills update-skills uninstall package-desktop clean setup

# ─── Version ────────────────────────────────────────────────────────────────
# Single source of truth for the build version (scripts/version.mjs); exported
# so the cargo build.rs files pick it up. CI sets FUTURE_VERSION on release tags.
FUTURE_VERSION_SCRIPT := $(CURDIR)/scripts/version.mjs
# Capture the public version before exporting the default +local display version.
# Explicit FUTURE_VERSION overrides are resolved by the packaging scripts instead.
ifeq ($(origin FUTURE_VERSION),undefined)
export FUTURE_ARTIFACT_VERSION ?= $(shell node "$(FUTURE_VERSION_SCRIPT)" --artifact-version)
endif
export FUTURE_VERSION ?= $(shell node "$(FUTURE_VERSION_SCRIPT)" || node -e "console.log('0.0.0-dev')" || echo 0.0.0-dev)

version:
	@node scripts/version.mjs --json

# ─── Platform ───────────────────────────────────────────────────────────────

TARGET := $(shell rustc -vV | node -e "process.stdin.on('data',d=>{const m=d.toString().match(/host:\s*(.+)/);if(m)console.log(m[1])})")
OS := $(word 3,$(subst -, ,$(TARGET)))
ifeq ($(OS),darwin)
  PREFIX := /opt/homebrew/bin
  SUDO :=
  EXE_SUFFIX :=
else ifeq ($(OS),linux)
  PREFIX := /usr/local/bin
  SUDO := sudo
  EXE_SUFFIX :=
else
  PREFIX := $(USERPROFILE)/.future/bin
  SUDO :=
  EXE_SUFFIX := .exe
endif

# Rust toolchain pinned by rust-toolchain.toml. rustup shims resolve it
# automatically; spelling it out keeps `-D warnings` clippy aligned with CI
# even when another cargo shadows the shim (e.g. Homebrew).
RUST_TOOLCHAIN := $(shell node -e "const m=require('fs').readFileSync('$(CURDIR)/rust-toolchain.toml','utf8').match(/channel\s*=\s*\"([^\"]+)/);console.log(m?m[1]:'stable')")
CARGO_PINNED := rustup run $(RUST_TOOLCHAIN) cargo

# ─── Install ────────────────────────────────────────────────────────────────
# The unified `future` CLI embeds agent/tui/channel/goal; the desktop bundles
# it for CLI tools and maintenance; Desktop hosts Agent in process by default.
# Standalone component binaries are dev-only. Full installs put `future` and
# `futureos` in PREFIX; `future host` runs the shared backend inside the CLI.

# One-time developer bootstrap for a fresh clone. Installs the workspace JS
# deps (shared packages, desktop, mobile) and the skills submodule,
# so any build / run / lint / test target below works
# without knowing which one installs what.
setup:
	git submodule update --init skills
	$(call npm-install-if-needed,desktop)
	$(call npm-install-if-needed,mobile)

install: install-desktop install-skills

# Copy already-built Windows binaries into PREFIX. make hands every recipe to
# SHELL, which is sh.exe whenever Git for Windows is on PATH, so this file's old
# cmd recipes (`if not exist .. mkdir ..`, unquoted `target\release\future.exe`)
# were a syntax error there — the documented install targets could not run on a
# Windows checkout with Git on PATH. node does the copy instead: it is already a
# build prerequisite (`TARGET` is probed with it) and its path joins and argument
# quoting behave the same under cmd.exe and sh.exe.
# $(1) is the profile directory under target/, $(2)/$(3) the binary names (the
# second is optional). It creates PREFIX itself, so an overridden PREFIX works too.
define copy_win_binaries
	@node -e "const fs=require('fs'),p=require('path');const src=p.join('target','$(1)'),dst=process.argv[1];fs.mkdirSync(dst,{recursive:true});for(const n of process.argv[2].split(/[\s,]+/).filter(Boolean)){const t=p.join(dst,n+'.exe');fs.copyFileSync(p.join(src,n+'.exe'),t);console.log('  installed '+t)}" "$(PREFIX)" "$(strip $(2) $(3))"
endef

# Copy the built-in skill bundles into the agent's skills directory on Windows.
# Same sh.exe reason as copy_win_binaries: `if not exist`, `for /d %%d`, `rmdir`
# and `xcopy` are cmd builtins, so with Git for Windows on PATH the recipe died
# with `syntax error: unexpected end of file from 'if' command` before copying
# anything. Each bundle is removed and copied afresh, which is the "replaced,
# not merged" result the cmd recipe gave and also what the POSIX branch's
# rm -rf + ln -s means; the check mark is a JS escape so the command line stays
# ASCII under every shell and code page. $(1) is the source directory, $(2) the
# destination.
define copy_win_skills
	@node -e "const fs=require('fs'),p=require('path');const [src,dst]=process.argv.slice(1);fs.mkdirSync(dst,{recursive:true});for(const n of fs.readdirSync(src)){const s=p.join(src,n),t=p.join(dst,n);if(!fs.statSync(s).isDirectory())continue;fs.rmSync(t,{recursive:true,force:true});fs.cpSync(s,t,{recursive:true});console.log('  '+String.fromCharCode(0x2713)+' '+n)}" "$(1)" "$(2)"
endef

install-cli: build-cli
ifeq ($(OS),windows)
	$(call copy_win_binaries,release,future)
else
	# `install` unlinks+creates a fresh inode; plain `cp` overwrites in place
	# and macOS taskgated can then SIGKILL the binary ("Code Signature
	# Invalid") because the vnode's cached signing state no longer matches.
	$(SUDO) install -m 755 target/release/future "$(PREFIX)/future"
endif

install-desktop: build-desktop
ifeq ($(OS),windows)
	$(call copy_win_binaries,release,future,futureos)
else
	$(SUDO) install -m 755 target/release/future "$(PREFIX)/future"
	$(SUDO) install -m 755 target/release/futureos "$(PREFIX)/futureos"
endif

# Local-profile install: build both binaries without LTO and put them in PREFIX,
# the same destinations install-cli/install-desktop use, so `future` on PATH and
# the Desktop agent (which prepends its own directory to PATH) run what you just
# built. This is not a release install — `make install*` keep owning PREFIX for
# shipping builds, and each overwrites the other. Windows cannot overwrite an
# image that is still running, so close any `future`/`futureos` started from
# PREFIX first. Skills are untouched; run `make install-skills` if you want them.
# The installed app is self-contained: `build-local` embeds the frontend into
# `futureos`, so it opens without a Vite dev server (see the build-local note).
#
# The Windows recipe copies through node rather than `copy`: make runs every
# recipe through SHELL, which is sh.exe whenever Git for Windows is on PATH, and
# cmd builtins plus backslash paths are not valid there. node is already a build
# prerequisite (`TARGET` is probed with it) and handles the quoting and
# directory creation in both shells.
install-local: build-local
ifeq ($(OS),windows)
	@node -e "const fs=require('fs'),p=require('path'),d=process.argv[1];fs.mkdirSync(d,{recursive:true});for(const n of ['future','futureos']){const t=p.join(d,n+'.exe');fs.copyFileSync(p.join('target','local',n+'.exe'),t);console.log('  installed '+t)}" "$(PREFIX)"
else
	# `install` unlinks+creates a fresh inode; plain `cp` overwrites in place
	# and macOS taskgated can then SIGKILL the binary ("Code Signature
	# Invalid") because the vnode's cached signing state no longer matches.
	$(SUDO) install -m 755 target/local/future "$(PREFIX)/future"
	$(SUDO) install -m 755 target/local/futureos "$(PREFIX)/futureos"
endif

# Compatibility alias for a GUI-free CLI installation, including headless.
install-desktop-headless: build-desktop-headless
ifeq ($(OS),windows)
	$(call copy_win_binaries,release,future)
else
	$(SUDO) install -m 755 target/release/future "$(PREFIX)/future"
endif

# Also removes pre-unification installs (future-agent/-tui/-channel).
#
# The Windows branch unlinks through node: `cmd /c del /q "..\future.exe" ..`
# looked like it worked under sh.exe (exit 0, the "Removed" line printed) but
# deleted nothing - bash hands cmd.exe the paths as separate argv entries, cmd
# drops to its prompt for the rest of the recipe's stdin, and make sees success.
# A missing file is skipped (`force`), while one held open by a running TUI is
# reported instead of failing the target, matching the old `-` prefix.
uninstall:
ifeq ($(OS),windows)
	@node -e "const fs=require('fs'),p=require('path');const dir=process.argv[1];for(const n of ['future','futureos','future-desktop','futureos-headless','future-agent','future-tui','future-channel']){const t=p.join(dir,n+'.exe');try{fs.rmSync(t)}catch(e){if(e.code!=='ENOENT')console.error('  skipped '+t+' ('+e.code+')')}}" "$(PREFIX)"
else
	$(SUDO) rm -f $(addprefix $(PREFIX)/,future futureos future-desktop futureos-headless future-agent future-tui future-channel)
endif
	@echo "Removed installed binaries from $(PREFIX)"

# Symlink the built-in skill bundles (skills/builtin/*, incl. future-goal)
# into the agent's skills directory. Only *orphaned* links are pruned: a link
# whose target no longer exists. Everything else in that directory is the
# user's, and is left alone — pruning by "is this name in skills/builtin?"
# also deleted skills the user installed from elsewhere (e.g. life-journal),
# which are linked from their own repository rather than from the submodule.
# A builtin whose directory the submodule drops (future-loop on the rename to
# future-goal) is a dangling link afterwards, so it is still pruned.
# Uses the submodule commits already checked out, so `make install` needs no
# network; `make update-skills` pulls the submodule's remote first.
install-skills:
	git submodule update --init skills
ifeq ($(OS),windows)
	$(call copy_win_skills,skills/builtin,$(USERPROFILE)/.future/agent/skills)
else
	@mkdir -p "$${HOME}/.future/agent/skills"
	@for skill_dir in skills/builtin/*/; do \
		name=$$(basename "$$skill_dir"); \
		rm -rf "$${HOME}/.future/agent/skills/$$name"; \
		ln -s "$$(cd "$$skill_dir" && pwd)" "$${HOME}/.future/agent/skills/$$name"; \
		echo "  ✓ $$name"; \
	done
	@for link in "$${HOME}/.future/agent/skills"/*; do \
		[ -L "$$link" ] || continue; \
		if [ ! -e "$$link" ]; then \
			name=$$(basename "$$link"); \
			rm -f "$$link"; \
			echo "  ✗ $$name (orphaned link removed)"; \
		fi; \
	done
endif

# Refresh the built-in skill bundles from the submodule's remote (the only
# skills step that needs the network), then install them.
update-skills:
	git submodule update --init --remote skills
	$(MAKE) install-skills

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-desktop

build-cli:
	cargo build --release -p future-cli

# Local verification only: the `local` profile (root Cargo.toml) keeps release
# optimisation but drops ThinLTO and reuses the incremental cache, so an edit
# rebuilds in seconds instead of re-running whole-graph LTO. These targets
# deliberately stay out of `install*` and `package-desktop`: what ships must
# keep coming from the unchanged `release` profile. Neither stages the Desktop
# sidecar nor bundles.
#
# The Desktop ones do build the frontend, because the binary they produce is not
# a dev-mode shell: `--features custom-protocol` (desktop/src-tauri/Cargo.toml)
# makes it serve `desktop/dist` from inside the binary instead of opening
# `build.devUrl`. Plain `cargo build` leaves Tauri on `cfg(dev)`, which is why a
# binary built without it shows ERR_CONNECTION_REFUSED for
# http://127.0.0.1:5173 unless a Vite server happens to be running. `npm run
# build` costs a few seconds and `stabilize-dist-mtimes` keeps an unchanged
# bundle's timestamps, so cargo still skips the Desktop crate when only Rust
# changed.
desktop-frontend:
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run build

build-cli-local:
	cargo build --profile local -p future-cli

build-desktop-local: desktop-frontend
	cargo build --profile local -p futureos --features custom-protocol

# Both binaries from one Cargo invocation, as `make install` does through
# scripts/build/build-desktop.mjs: one dependency graph, so shared crates build
# once and feature unification matches the shipping build. Writes
# target/local/{future,futureos} only — it does not install into PREFIX or
# bundle; those stay with the release path (`make install*`,
# `make package-desktop`).
build-local: desktop-frontend
	cargo build --profile local -p future-cli -p futureos --bin future --bin futureos --features custom-protocol

# Workspace install: shared packages, desktop, and mobile are npm workspaces, so all
# deps hoist to the root node_modules and one `npm install` at the repo root
# installs everything. Run it only when a manifest is newer than npm's own
# install stamp — scripts/npm-install-if-needed.mjs checks the whole tree, so the
# workspace the callers name is informational only (npm install is one global
# operation). cmd/make has no usable mtime test, so Windows used to run a full
# `npm install` on every build.
define npm-install-if-needed
	@node scripts/npm-install-if-needed.mjs
endef

# Stage the unified `future` CLI as the Tauri sidecar (externalBin).
#
# The Windows branch copies through node for the reason the install targets do:
# make hands every recipe to SHELL, which is sh.exe whenever Git for Windows is
# on PATH, and this target used to write `if not exist .. mkdir ..` plus
# `copy /y target\release\future.exe` - cmd builtins and backslash paths that
# bash either rejects outright or strips the separators out of. node is already
# a build prerequisite (`TARGET` is probed with it) and joins/quotes identically
# under both shells. The POSIX branch is unchanged.
desktop-sidecars: build-cli
ifeq ($(OS),windows)
	@node -e "const fs=require('fs'),p=require('path');const dst=p.join('desktop','src-tauri','binaries');fs.mkdirSync(dst,{recursive:true});const t=p.join(dst,'future-'+process.argv[1]+'.exe');fs.copyFileSync(p.join('target','release','future.exe'),t);console.log('  staged '+t)" "$(TARGET)"
else
	@mkdir -p desktop/src-tauri/binaries
	install -m 755 target/release/future desktop/src-tauri/binaries/future-$(TARGET)
endif

# Self-contained standalone binary (no installer):
#   target/release/futureos$(EXE_SUFFIX)
# `tauri build` runs the frontend build (`npm run build`) via beforeBuildCommand,
# so it is not repeated here.
build-desktop:
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run tauri:build -- --no-bundle

# Compatibility target: GUI-free `future host`, with no npm/Tauri build.
build-desktop-headless:
	$(CARGO_PINNED) build --release -p future-cli --bin future

# Mobile native projects are generated locally by Expo (gitignored).
build-mobile-android:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run android

build-mobile-ios:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run ios

# ─── Test ───────────────────────────────────────────────────────────────────
# The unit tests are the CI regression gate. The *-diff / *-tmux targets are
# manual TS→Rust migration-acceptance gates (pre-release only).

test: test-agent test-channels test-cli test-app-settings test-tui test-desktop test-desktop-rust test-mobile

test-agent:
	cargo test -p future-agent

test-channels:
	cargo test -p future-channel

test-cli:
	cargo test -p future-cli

test-app-settings:
	cargo test -p future-app-settings

test-tui:
	cargo test -p future-tui

test-cli-diff:
	./cli/tests/golden-diff.sh

test-tui-diff:
	./tui/tests/golden-diff.sh

test-tui-tmux:
	./tui/tests/tmux-diff.sh

test-desktop:
	$(call npm-install-if-needed,desktop)
	cd desktop && npm test

test-desktop-rust:
	cd desktop/src-tauri && cargo test

test-mobile:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm test

# ─── Lint ───────────────────────────────────────────────────────────────────
# lint-rust mirrors the CI job exactly (workspace + desktop backend).

lint: lint-rust lint-desktop stylelint-desktop lint-mobile

lint-rust:
	$(CARGO_PINNED) fmt --check --all
	$(CARGO_PINNED) clippy --workspace --all-targets -- -D warnings
	$(CARGO_PINNED) clippy -p future-host -p future-cli --all-targets -- -D warnings

lint-desktop:
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run lint

stylelint-desktop:
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run stylelint

lint-mobile:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run typecheck && npm run lint

check-desktop: lint-desktop stylelint-desktop
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run build
	cd desktop/src-tauri && cargo check

check-mobile: lint-mobile test-mobile

fmt:
	cargo fmt --all

# ─── Run ────────────────────────────────────────────────────────────────────
# Kept as `cd <crate> && cargo run` on purpose: the process cwd is user-visible
# (TUI footer, goal status root).

# Bare --log-file (no value) enables file logging at the default location,
# ~/.future/agent/logs/agent.log; console output stays on the terminal.
run-agent:
	cd agent && cargo run -- --verbose --log-file

run-tui:
	cd tui && cargo run

run-cli:
	cd cli && cargo run

# `run-host` is the name; `run-headless` is kept as an alias for muscle memory.
run-host:
	$(CARGO_PINNED) run -p future-cli --bin future -- host $(ARGS)

run-headless: run-host

run-channels:
	cd channels && cargo run

run-goal:
	cd orchestration/goal && cargo run

run-desktop:
ifeq ($(OS),windows)
	cmd /c scripts\dev\start-desktop-windows.bat
else ifeq ($(OS),darwin)
	bash scripts/dev/start-desktop-macos.sh
else
	bash scripts/dev/start-desktop-linux.sh
endif

run-mobile-android:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run android:device

run-mobile-ios:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run ios

# ─── Profile ────────────────────────────────────────────────────────────────
# profile-agent-build: release agent with frame pointers + line tables so
# profilers resolve symbols (shared by profile-agent / profile-quick).
#
# The overrides are target-specific *exported* make variables rather than the
# `set "CARGO_.." &&` (cmd) or `CARGO_..=.. cargo` (POSIX) prefixes they used to
# be: the Windows form is a syntax error under sh.exe, which is the shell make
# actually uses whenever Git for Windows is on PATH, and the POSIX form is one
# under cmd.exe. An exported make variable is in the recipe's environment
# whatever the shell is, and cargo reads all three from there - including the
# profile keys, through `CARGO_PROFILE_<name>_<key>`. That also removes the
# `--config` arguments, whose quoting had to differ per shell
# (`--config "profile.release.debug=""line-tables-only"""` vs
# `--config 'profile.release.debug="line-tables-only"'`).

profile-agent-build: export RUSTFLAGS := -C force-frame-pointers=yes
profile-agent-build: export CARGO_PROFILE_RELEASE_DEBUG := line-tables-only
profile-agent-build: export CARGO_PROFILE_RELEASE_STRIP := none
profile-agent-build:
	cargo build --release -p future-agent

# CPU profile: build + 90s bench, write flamegraph SVG to profile-results/.
#
# `PROFILE_DURATION` is exported make state for the same reason: the POSIX form
# of this line (`PROFILE_DURATION=90 python3 ..`) is not a command cmd.exe can
# run, and the Windows form (`set "PROFILE_DURATION=90" && python ..`) is not
# one sh.exe can. profile-results/ is created with node, whose mkdir is the same
# call under both shells (cmd has no `mkdir -p`, and `if not exist .. mkdir ..`
# is the syntax error above). The blondie check is a node spawn for the same
# reason as the rest: `where blondie >NUL || (echo. & .. & exit /b 1)` is cmd
# syntax in its redirection, its grouping and its exit. The progress line goes
# through node too: unquoted, its parentheses ended the shell word and sh.exe
# rejected the line (`syntax error near unexpected token '('`); quoted, cmd's
# `echo` would print the quotes.
profile-agent: export PROFILE_DURATION := 90
profile-agent: profile-agent-build
ifeq ($(OS),windows)
	@node -e "require('fs').mkdirSync('profile-results',{recursive:true})"
	@node -e "const r=require('child_process').spawnSync('where',['blondie'],{stdio:'ignore'});if(r.status!==0){console.error('');console.error('  blondie is required for CPU profiling on Windows.');console.error('  Install: cargo install blondie --features inferno');console.error('  CPU profiling also requires administrator privileges.');process.exit(1)}"
	@node -e "console.log('Starting profile run (port 50052, 90s)...')"
	python scripts/measure/profile-isolated.py powershell -ExecutionPolicy Bypass -File scripts/measure/agent-profile-bench.ps1
else
	@mkdir -p profile-results
	@echo "Starting profile run (port 50052, 90s)..."
	python3 scripts/measure/profile-isolated.py bash scripts/measure/agent-profile-bench.sh
	@echo "Flamegraph: $$(ls -t profile-results/agent-profile-*.svg | head -1)"
endif

# Quick CPU profile: run agent N seconds. Usage: make profile-quick PROFILE_SECS=30
profile-quick: profile-agent-build
ifeq ($(OS),windows)
	python scripts/measure/profile-isolated.py powershell -ExecutionPolicy Bypass -File scripts/measure/profile-quick.ps1 -Duration $(or $(PROFILE_SECS),30)
else
	@mkdir -p profile-results
	python3 scripts/measure/profile-isolated.py ./target/release/future-agent \
		--grpc-addr 127.0.0.1:50052 \
		--profile profile-results/quick-profile.svg \
		--profile-seconds $(or $(PROFILE_SECS),30) \
		--verbose
endif

# Heap profile via dhat (needs full debug info on Windows). View the report at
# https://nnethercote.github.io/dh_view/dh_view.html
# Usage: make profile-heap PROFILE_SECS=30
#
# Same exported-variable and node-mkdir reasoning as profile-agent-build.
profile-heap: export CARGO_PROFILE_RELEASE_STRIP := none
ifeq ($(OS),windows)
profile-heap: export CARGO_PROFILE_RELEASE_DEBUG := 2
else
profile-heap: export CARGO_PROFILE_RELEASE_DEBUG := line-tables-only
endif
profile-heap:
	cargo build --release -p future-agent --features dhat-heap
ifeq ($(OS),windows)
	@node -e "require('fs').mkdirSync('profile-results',{recursive:true})"
else
	@mkdir -p profile-results
endif
	$(if $(filter windows,$(OS)),python,python3) scripts/measure/profile-isolated.py ./target/release/future-agent$(EXE_SUFFIX) \
		--grpc-addr 127.0.0.1:50052 \
		--profile-heap profile-results/heap-profile.json \
		--profile-seconds $(or $(PROFILE_SECS),30) \
		--verbose
	@echo "Heap profile: profile-results/heap-profile.json"

# ─── Generate ───────────────────────────────────────────────────────────────

generate-models:
	python3 scripts/docs/generate_models.py

# Documentation gate. Not wired into `make lint` or CI on purpose: nothing runs
# it automatically yet, so it is a deliberate, separate decision to add it to a
# pipeline. Run it before touching docs/.
check-docs:
	python3 scripts/docs/check-docs.py
	python3 scripts/docs/check-docs.py --strict-pending
	python3 scripts/docs/check-channel-matrix.py

# Regression tests for the documentation gate itself (includes negative
# controls, so a checker that stopped detecting violations would fail here).
#
# `PYTHONDONTWRITEBYTECODE` is declared as a target-specific exported variable
# rather than written as a `VAR=1 command` prefix. That prefix is POSIX shell
# syntax: a Windows make runs recipes through cmd.exe, which takes the first word
# as the program to run (`'PYTHONDONTWRITEBYTECODE' is not recognized …`). An
# exported make variable is placed in the recipe's environment whatever the
# shell is, and the interpreter's own children inherit it — which is the part a
# bare `python3 -B` would not do for the subprocesses this test spawns.
test-docs-check: export PYTHONDONTWRITEBYTECODE := 1
test-docs-check:
	python3 scripts/tests/test-check-docs.py

# Regression tests for the screenshot harness: the browser lifecycle (the startup
# sweep of browsers/profiles a killed run left behind, and the rule that only a
# headless browser is ever driven — starts real headless Chrome, so those cases
# skip where none is installed), and the upright check's judge for mobile
# captures of the inverted transcript (offline: it draws the tiles it judges).
# Same target-specific export as `test-docs-check`, for the same reason.
test-screenshot-harness: export PYTHONDONTWRITEBYTECODE := 1
test-screenshot-harness:
	python3 scripts/tests/test-screenshot-harness-reap.py
	python3 scripts/tests/test-screenshot-upright-check.py
	python3 scripts/tests/test-screenshot-scenario-fragments.py

# Wire codegen owners: packages/rpc (future.proto) + channels (feishu_ws pbbp2).
#
# `REGENERATE_PROTO` is a target-specific exported variable, not a `VAR=1 cmd`
# prefix: that prefix is POSIX shell syntax, and a Windows make runs recipes
# through cmd.exe, which reads the first word as the program to run. Cargo's
# build script is what consumes the variable (`packages/rpc/build.rs`), and it
# receives it from the environment either way.
generate-proto: export REGENERATE_PROTO := 1
generate-proto:
	cd packages/rpc && cargo build
	cd channels && cargo build

# ─── Clean ──────────────────────────────────────────────────────────────────
# Build artifacts only — installed binaries are removed by `uninstall`.
#
# The Windows branch deletes through node for the same reason the install
# targets copy through it: `@if exist target rmdir /s /q target` is a syntax
# error under sh.exe (`unexpected end of file from 'if' command`), so `make
# clean` could not run at all on a Windows checkout with Git on PATH.
# `fs.rmSync(dir, {recursive:true, force:true})` is what `rmdir /s /q` did:
# a missing path is skipped, contents go with the directory, and a file still
# held open fails loudly instead of leaving half a tree behind in silence. It
# also removes a directory junction rather than what it points at, so a
# worktree that linked the main checkout's node_modules does not empty it.
clean:
ifeq ($(OS),windows)
	@node -e "const fs=require('fs');for(const d of ['target','desktop/dist','desktop/node_modules','desktop/src-tauri/target','desktop/src-tauri/binaries'])fs.rmSync(d,{recursive:true,force:true})"
else
	rm -rf target desktop/dist desktop/node_modules desktop/src-tauri/target desktop/src-tauri/binaries
endif

# ─── Package ────────────────────────────────────────────────────────────────

package-desktop:
	$(call npm-install-if-needed,desktop)
ifeq ($(OS),windows)
	powershell -ExecutionPolicy Bypass -File scripts/build/build-desktop-windows-installer.ps1 -SkipDeps
else ifeq ($(OS),darwin)
	bash scripts/build/build-desktop-macos.sh --skip-deps
else
	bash scripts/build/build-desktop-linux.sh --skip-deps
endif

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build / build-cli / build-desktop   Build desktop + unified CLI (or each part)"
	@echo "  build-cli-local / build-desktop-local   LTO-free 'local' profile binaries (the Desktop one embeds the frontend)"
	@echo "  build-local                         Both binaries (future + futureos) in one 'local' profile build"
	@echo "  run-host                          Run future host in the foreground (ARGS=...; alias run-headless)"
	@echo "  build-desktop-headless             Build GUI-free CLI with headless"
	@echo "  build-mobile-android / -ios         Build & install the mobile app (Expo)"
	@echo "  test                                All unit tests (Rust crates + desktop + mobile)"
	@echo "  test-<crate>                        test-agent / -channels / -cli / -tui / -desktop(-rust) / -mobile"
	@echo "  test-cli-diff / -tui-diff / -tui-tmux  [manual] TS→Rust migration gates"
	@echo "  lint                                Rust (CI flags) + desktop + mobile lints"
	@echo "  check-desktop / check-mobile        Lint + typecheck + tests without building apps"
	@echo "  check-docs / test-docs-check        Docs placement/pairing/links gate + its regression tests"
	@echo "  fmt                                 Format Rust code"
	@echo "  run-agent / -tui / -cli / -channels / -goal   Run a component (debug build)"
	@echo "  run-desktop / run-mobile-android / run-mobile-ios   Run an app in dev mode"
	@echo "  profile-agent / profile-quick / profile-heap  CPU/heap profiling (PROFILE_SECS=30)"
	@echo "  generate-models                     Fetch model data, regenerate Rust catalog + wiki docs"
	@echo "  generate-proto                      Regenerate wire code (packages/rpc future.proto + channels feishu_ws)"
	@echo "  install / install-cli / install-desktop / install-desktop-headless / install-skills   Install to $(PREFIX)"
	@echo "  install-local                       Install the LTO-free 'local' binaries to $(PREFIX) (fast local iteration)"
	@echo "  update-skills                       Pull the skills submodule's remote, then install skills"
	@echo "  setup                               Bootstrap a fresh clone (JS deps + skills + sidecar)"
	@echo "  uninstall                           Remove installed binaries from $(PREFIX)"
	@echo "  package-desktop                     Package desktop bundles"
	@echo "  clean                               Remove build artifacts"
