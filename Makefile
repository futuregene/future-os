.PHONY: help version \
	build build-cli build-desktop build-desktop-headless build-mobile-android build-mobile-ios desktop-sidecars \
	test test-agent test-channels test-cli test-app-settings test-tui test-cli-diff test-tui-diff test-tui-tmux \
	test-desktop test-desktop-rust test-mobile \
	lint lint-rust lint-desktop stylelint-desktop lint-mobile check-desktop check-mobile fmt \
	check-docs test-docs-check \
	run-agent run-tui run-cli run-desktop run-mobile-android run-mobile-ios run-channels run-goal \
	profile-agent-build profile-agent profile-quick profile-heap \
	generate-models generate-proto \
	install install-cli install-desktop install-desktop-headless install-skills update-skills uninstall package-desktop clean setup

# ─── Version ────────────────────────────────────────────────────────────────
# Single source of truth for the build version (scripts/version.mjs); exported
# so the cargo build.rs files pick it up. CI sets FUTURE_VERSION on release tags.
FUTURE_VERSION_SCRIPT := $(CURDIR)/scripts/version.mjs
export FUTURE_VERSION ?= $(shell node "$(FUTURE_VERSION_SCRIPT)" || node -e "console.log('0.0.0-dev')" || echo 0.0.0-dev)

version:
	@node scripts/version.mjs --json

# ─── Platform ───────────────────────────────────────────────────────────────

TARGET := $(shell rustc -vV | node -e "process.stdin.on('data',d=>{const m=d.toString().match(/host:\s*(.+)/);if(m)console.log(m[1])})")
OS := $(word 3,$(subst -, ,$(TARGET)))
ifeq ($(OS),darwin)
  PREFIX := /opt/homebrew/bin
  SUDO :=
  COPY_CMD := cp
  EXE_SUFFIX :=
else ifeq ($(OS),linux)
  PREFIX := /usr/local/bin
  SUDO := sudo
  COPY_CMD := cp
  EXE_SUFFIX :=
else
  PREFIX := $(USERPROFILE)/.future/bin
  SUDO :=
  COPY_CMD := cmd /c copy /y
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
# Standalone future-agent/-tui/-channel/-goal
# binaries are dev-only (cargo run -p <crate>) — install puts three files in
# PREFIX: `future`, `future-desktop`, and the GUI-free `futureos-headless`.
# The headless server keeps its release-package name (Linux .deb and portable
# tarball install `futureos-headless` too), and its own help text names it
# that way; it finds `future` beside it for skill initialisation.

# One-time developer bootstrap for a fresh clone. Installs the workspace JS
# deps (shared packages, desktop, mobile) and the skills submodule,
# so any build / run / lint / test target below works
# without knowing which one installs what.
setup:
	git submodule update --init skills
	$(call npm-install-if-needed,desktop)
	$(call npm-install-if-needed,mobile)

install: install-desktop install-desktop-headless install-skills

install-cli: build-cli
ifeq ($(OS),windows)
	@if not exist "$(USERPROFILE)\.future\bin" mkdir "$(USERPROFILE)\.future\bin"
	$(COPY_CMD) target\release\future$(EXE_SUFFIX) "$(PREFIX)\future$(EXE_SUFFIX)"
else
	# `install` unlinks+creates a fresh inode; plain `cp` overwrites in place
	# and macOS taskgated can then SIGKILL the binary ("Code Signature
	# Invalid") because the vnode's cached signing state no longer matches.
	$(SUDO) install -m 755 target/release/future "$(PREFIX)/future"
endif

install-desktop: build-desktop
ifeq ($(OS),windows)
	@if not exist "$(USERPROFILE)\.future\bin" mkdir "$(USERPROFILE)\.future\bin"
	$(COPY_CMD) target\release\future$(EXE_SUFFIX) "$(PREFIX)\future$(EXE_SUFFIX)"
	$(COPY_CMD) target\release\futureos$(EXE_SUFFIX) "$(PREFIX)\future-desktop$(EXE_SUFFIX)"
else
	$(SUDO) install -m 755 target/release/future "$(PREFIX)/future"
	$(SUDO) install -m 755 target/release/futureos "$(PREFIX)/future-desktop"
endif

# GUI-free server, plus the matching CLI it looks for beside itself. The server
# keeps its release-package name (the Linux .deb and portable tarball install
# `futureos-headless` too), and its own help text names it that way.
install-desktop-headless: build-desktop-headless
ifeq ($(OS),windows)
	@if not exist "$(USERPROFILE)\.future\bin" mkdir "$(USERPROFILE)\.future\bin"
	$(COPY_CMD) target\release\future$(EXE_SUFFIX) "$(PREFIX)\future$(EXE_SUFFIX)"
	$(COPY_CMD) target\release\futureos-headless$(EXE_SUFFIX) "$(PREFIX)\futureos-headless$(EXE_SUFFIX)"
else
	$(SUDO) install -m 755 target/release/future "$(PREFIX)/future"
	$(SUDO) install -m 755 target/release/futureos-headless "$(PREFIX)/futureos-headless"
endif

# Also removes pre-unification installs (future-agent/-tui/-channel).
uninstall:
ifeq ($(OS),windows)
	-cmd /c del /q "$(PREFIX)\future$(EXE_SUFFIX)" "$(PREFIX)\future-desktop$(EXE_SUFFIX)" "$(PREFIX)\futureos-headless$(EXE_SUFFIX)" "$(PREFIX)\future-agent$(EXE_SUFFIX)" "$(PREFIX)\future-tui$(EXE_SUFFIX)" "$(PREFIX)\future-channel$(EXE_SUFFIX)" 2>NUL
else
	$(SUDO) rm -f $(addprefix $(PREFIX)/,future future-desktop futureos-headless future-agent future-tui future-channel)
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
	@if not exist "$(USERPROFILE)\.future\agent\skills" mkdir "$(USERPROFILE)\.future\agent\skills"
	@for /d %%d in (skills\builtin\*) do @( \
		rmdir /s /q "$(USERPROFILE)\.future\agent\skills\%%~nxd" 2>NUL & \
		xcopy /e /i /y "%%d" "$(USERPROFILE)\.future\agent\skills\%%~nxd" >NUL & \
		echo   ✓ %%~nxd \
	)
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
desktop-sidecars: build-cli
ifeq ($(OS),windows)
	@if not exist desktop\src-tauri\binaries mkdir desktop\src-tauri\binaries
	$(COPY_CMD) target\release\future$(EXE_SUFFIX) "desktop\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)"
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

# GUI-free server binary plus the matching Agent/CLI, with no npm/Tauri build.
build-desktop-headless:
	$(CARGO_PINNED) build --release -p future-cli -p futureos --no-default-features --features futureos/headless --bin future --bin futureos-headless

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
	$(CARGO_PINNED) clippy -p futureos --no-default-features --features headless --all-targets -- -D warnings

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

profile-agent-build:
ifeq ($(OS),windows)
	set "RUSTFLAGS=-C force-frame-pointers=yes" && cargo build --release -p future-agent \
		--config "profile.release.debug=""line-tables-only""" --config "profile.release.strip=""none"""
else
	RUSTFLAGS="-C force-frame-pointers=yes" cargo build --release -p future-agent \
		--config 'profile.release.debug="line-tables-only"' --config 'profile.release.strip="none"'
endif

# CPU profile: build + 90s bench, write flamegraph SVG to profile-results/.
profile-agent: profile-agent-build
ifeq ($(OS),windows)
	@if not exist profile-results mkdir profile-results
	@where blondie >NUL 2>NUL || (echo. & echo  blondie is required for CPU profiling on Windows. & echo  Install: cargo install blondie --features inferno & echo  CPU profiling also requires administrator privileges. & exit /b 1)
	@echo Starting profile run (port 50052, 90s)...
	set "PROFILE_DURATION=90" && python scripts/measure/profile-isolated.py powershell -ExecutionPolicy Bypass -File scripts/measure/agent-profile-bench.ps1
else
	@mkdir -p profile-results
	@echo "Starting profile run (port 50052, 90s)..."
	PROFILE_DURATION=90 python3 scripts/measure/profile-isolated.py bash scripts/measure/agent-profile-bench.sh
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
profile-heap:
ifeq ($(OS),windows)
	cargo build --release -p future-agent --features dhat-heap \
		--config profile.release.debug=2 --config "profile.release.strip=""none"""
	@if not exist profile-results mkdir profile-results
else
	cargo build --release -p future-agent --features dhat-heap \
		--config 'profile.release.debug="line-tables-only"' --config 'profile.release.strip="none"'
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
test-docs-check:
	PYTHONDONTWRITEBYTECODE=1 python3 scripts/tests/test-check-docs.py

# Regression tests for the screenshot harness: the browser lifecycle (the startup
# sweep of browsers/profiles a killed run left behind, and the rule that only a
# headless browser is ever driven — starts real headless Chrome, so those cases
# skip where none is installed), and the upright check's judge for mobile
# captures of the inverted transcript (offline: it draws the tiles it judges).
test-screenshot-harness:
	PYTHONDONTWRITEBYTECODE=1 python3 scripts/tests/test-screenshot-harness-reap.py
	PYTHONDONTWRITEBYTECODE=1 python3 scripts/tests/test-screenshot-upright-check.py

# Wire codegen owners: packages/rpc (future.proto) + channels (feishu_ws pbbp2).
generate-proto:
	cd packages/rpc && REGENERATE_PROTO=1 cargo build
	cd channels && REGENERATE_PROTO=1 cargo build

# ─── Clean ──────────────────────────────────────────────────────────────────
# Build artifacts only — installed binaries are removed by `uninstall`.

clean:
ifeq ($(OS),windows)
	@if exist target rmdir /s /q target
	@if exist desktop\dist rmdir /s /q desktop\dist
	@if exist desktop\node_modules rmdir /s /q desktop\node_modules
	@if exist desktop\src-tauri\target rmdir /s /q desktop\src-tauri\target
	@if exist desktop\src-tauri\binaries rmdir /s /q desktop\src-tauri\binaries
else
	rm -rf target desktop/dist desktop/node_modules desktop/src-tauri/target desktop/src-tauri/binaries
endif

# ─── Package ────────────────────────────────────────────────────────────────

package-desktop:
	$(call npm-install-if-needed,desktop)
	node scripts/version.mjs --set-bundle
	cd desktop && npm run tauri:build

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build / build-cli / build-desktop   Build desktop + unified CLI (or each part)"
	@echo "  build-desktop-headless             Build GUI-free server + matching CLI"
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
	@echo "  update-skills                       Pull the skills submodule's remote, then install skills"
	@echo "  setup                               Bootstrap a fresh clone (JS deps + skills + sidecar)"
	@echo "  uninstall                           Remove installed binaries from $(PREFIX)"
	@echo "  package-desktop                     Package desktop bundles"
	@echo "  clean                               Remove build artifacts"
