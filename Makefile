.PHONY: help version \
	build build-cli build-desktop build-mobile-android build-mobile-ios desktop-sidecars build-thread-projection \
	test test-agent test-channels test-cli test-tui test-cli-diff test-tui-diff test-tui-tmux \
	test-desktop test-desktop-rust test-mobile \
	lint lint-rust lint-desktop stylelint-desktop lint-mobile check-desktop check-mobile fmt fmt-mobile \
	run-agent run-tui run-cli run-desktop run-mobile-android run-mobile-ios run-channels run-loop \
	profile-agent-build profile-agent profile-quick profile-heap \
	generate-models generate-proto \
	install install-cli install-desktop install-skills uninstall package-desktop clean setup

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
# The unified `future` CLI embeds agent/tui/channel/loop; the desktop bundles
# it as a sidecar (`future agent`). Standalone future-agent/-tui/-channel/-loop
# binaries are dev-only (cargo run -p <crate>) — install puts exactly two files
# in PREFIX: `future` and `future-desktop`.

# One-time developer bootstrap for a fresh clone. Installs the shared
# thread-projection deps (via build-thread-projection), the desktop/mobile JS
# deps, the skills submodule, and an empty sidecar placeholder — so any build /
# run / lint / test target below works without knowing which one installs what.
setup: build-thread-projection
	git submodule update --init skills
	$(call npm-install-if-needed,desktop)
	$(call npm-install-if-needed,mobile)
	$(MAKE) desktop-sidecar-placeholder

install: install-cli install-desktop install-skills

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

install-desktop: install-cli desktop-sidecars
	$(call npm-install-if-needed,desktop)
	cd desktop && npx tauri build --no-bundle
ifeq ($(OS),windows)
	$(COPY_CMD) desktop\src-tauri\target\release\futureos$(EXE_SUFFIX) "$(PREFIX)\future-desktop$(EXE_SUFFIX)"
else
	$(SUDO) install -m 755 desktop/src-tauri/target/release/futureos "$(PREFIX)/future-desktop"
endif

# Also removes pre-unification installs (future-agent/-tui/-channel).
uninstall:
ifeq ($(OS),windows)
	-cmd /c del /q "$(PREFIX)\future$(EXE_SUFFIX)" "$(PREFIX)\future-desktop$(EXE_SUFFIX)" "$(PREFIX)\future-agent$(EXE_SUFFIX)" "$(PREFIX)\future-tui$(EXE_SUFFIX)" "$(PREFIX)\future-channel$(EXE_SUFFIX)" 2>NUL
else
	$(SUDO) rm -f $(addprefix $(PREFIX)/,future future-desktop future-agent future-tui future-channel)
endif
	@echo "Removed installed binaries from $(PREFIX)"

# Symlink the built-in skill bundles (skills/builtin/*, incl. future-loop)
# into the agent's skills directory (orphaned links are pruned).
install-skills:
	git submodule update --init --remote skills
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
		name=$$(basename "$$link"); \
		if [ ! -d "skills/builtin/$$name" ]; then \
			rm -rf "$$link"; \
			echo "  ✗ $$name (removed)"; \
		fi; \
	done
endif

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-cli build-desktop

build-cli:
	cargo build --release -p future-cli

# Only run npm install when package.json is newer than the install stamp
# (on Windows npm install is idempotent — just run it).
ifeq ($(OS),windows)
define npm-install-if-needed
	@cd $(1) && npm install --silent
endef
else
define npm-install-if-needed
	@if [ ! -f "$(1)/node_modules/.package-lock.json" ] || [ "$(1)/package.json" -nt "$(1)/node_modules/.package-lock.json" ]; then \
		echo "  npm install $(1)/"; \
		cd $(1) && npm install; \
	fi
endef
endif

# Stage the unified `future` CLI as the Tauri sidecar (externalBin).
desktop-sidecars: build-cli
ifeq ($(OS),windows)
	@if not exist desktop\src-tauri\binaries mkdir desktop\src-tauri\binaries
	$(COPY_CMD) target\release\future$(EXE_SUFFIX) "desktop\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)"
else
	@mkdir -p desktop/src-tauri/binaries
	cp target/release/future desktop/src-tauri/binaries/future-$(TARGET)
endif

# Empty sidecar placeholder: tauri-build aborts when the externalBin path is
# missing, even for check/clippy/test. CI does the same (see ci.yml).
desktop-sidecar-placeholder:
ifeq ($(OS),windows)
	@if not exist "desktop\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)" (if not exist desktop\src-tauri\binaries mkdir desktop\src-tauri\binaries & type NUL > "desktop\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)")
else
	@mkdir -p desktop/src-tauri/binaries
	@[ -f "desktop/src-tauri/binaries/future-$(TARGET)" ] || : > "desktop/src-tauri/binaries/future-$(TARGET)"
endif

# Shared thread projection package: desktop/mobile both depend on its compiled
# dist/ via a `file:` dep, so any TS change here must rebuild before either app
# typechecks or runs. Every desktop/mobile build, test, lint and run target
# below depends on this, so it always reflects the latest source — no manual step.
# The stale-check lives in helper scripts: make recipes run under cmd.exe on
# Windows, which cannot parse POSIX shell (`if [ ! -f ... ]`).
build-thread-projection:
ifeq ($(OS),windows)
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-thread-projection.ps1
else
	bash scripts/build-thread-projection.sh
endif

# Self-contained standalone binary (no installer):
#   desktop/src-tauri/target/release/futureos$(EXE_SUFFIX)
# `tauri build` runs the frontend build (`npm run build`) via beforeBuildCommand,
# so it is not repeated here.
build-desktop: build-thread-projection desktop-sidecars
	$(call npm-install-if-needed,desktop)
	cd desktop && npx tauri build --no-bundle

# Mobile native projects are generated locally by Expo (gitignored).
build-mobile-android: build-thread-projection
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run android

build-mobile-ios: build-thread-projection
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run ios

# ─── Test ───────────────────────────────────────────────────────────────────
# The unit tests are the CI regression gate. The *-diff / *-tmux targets are
# manual TS→Rust migration-acceptance gates (pre-release only).

test: test-agent test-channels test-cli test-tui test-desktop test-desktop-rust test-mobile

test-agent:
	cargo test -p future-agent

test-channels:
	cargo test -p future-channel

test-cli:
	cargo test -p future-cli

test-tui:
	cargo test -p future-tui

test-cli-diff:
	./cli/tests/golden-diff.sh

test-tui-diff:
	./tui/tests/golden-diff.sh

test-tui-tmux:
	./tui/tests/tmux-diff.sh

test-desktop: build-thread-projection
	$(call npm-install-if-needed,desktop)
	cd desktop && npm test

test-desktop-rust: desktop-sidecar-placeholder
	cd desktop/src-tauri && cargo test

test-mobile: build-thread-projection
	$(call npm-install-if-needed,mobile)
	cd mobile && npm test

# ─── Lint ───────────────────────────────────────────────────────────────────
# lint-rust mirrors the CI job exactly (workspace + desktop backend).

lint: lint-rust lint-desktop stylelint-desktop lint-mobile

lint-rust: desktop-sidecar-placeholder
	$(CARGO_PINNED) fmt --check --all
	$(CARGO_PINNED) clippy --workspace --all-targets -- -D warnings
	$(CARGO_PINNED) fmt --check --manifest-path desktop/src-tauri/Cargo.toml
	$(CARGO_PINNED) clippy --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings

lint-desktop: build-thread-projection
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run lint

stylelint-desktop:
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run stylelint

lint-mobile: build-thread-projection
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run typecheck && npm run lint

check-desktop: lint-desktop stylelint-desktop build-thread-projection desktop-sidecar-placeholder
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run build
	cd desktop/src-tauri && cargo check

check-mobile: lint-mobile test-mobile
	cd mobile && npm run format:check

fmt:
	cargo fmt --all
	cargo fmt --manifest-path desktop/src-tauri/Cargo.toml
	$(MAKE) fmt-mobile

fmt-mobile:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run format

# ─── Run ────────────────────────────────────────────────────────────────────
# Kept as `cd <crate> && cargo run` on purpose: the process cwd is user-visible
# (TUI footer, loop status root).

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

run-loop:
	cd orchestration/loop && cargo run

run-desktop: build-thread-projection desktop-sidecars
	$(call npm-install-if-needed,desktop)
	cd desktop && npm run tauri:dev

run-mobile-android: build-thread-projection
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run android:device

run-mobile-ios: build-thread-projection
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
	set "PROFILE_DURATION=90" && powershell -ExecutionPolicy Bypass -File scripts/agent-profile-bench.ps1
else
	@mkdir -p profile-results
	@echo "Starting profile run (port 50052, 90s)..."
	PROFILE_DURATION=90 bash scripts/agent-profile-bench.sh
	@echo "Flamegraph: $$(ls -t profile-results/agent-profile-*.svg | head -1)"
endif

# Quick CPU profile: run agent N seconds. Usage: make profile-quick PROFILE_SECS=30
profile-quick: profile-agent-build
ifeq ($(OS),windows)
	powershell -ExecutionPolicy Bypass -File scripts/profile-quick.ps1 -Duration $(or $(PROFILE_SECS),30)
else
	@mkdir -p profile-results
	./target/release/future-agent \
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
	./target/release/future-agent$(EXE_SUFFIX) \
		--grpc-addr 127.0.0.1:50052 \
		--profile-heap profile-results/heap-profile.json \
		--profile-seconds $(or $(PROFILE_SECS),30) \
		--verbose
	@echo "Heap profile: profile-results/heap-profile.json"

# ─── Generate ───────────────────────────────────────────────────────────────

generate-models:
	python3 scripts/generate_models.py

# Wire codegen owners: rpc (future.proto) + channels (feishu_ws pbbp2).
generate-proto:
	cd rpc && REGENERATE_PROTO=1 cargo build
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

package-desktop: install-desktop
	node scripts/version.mjs --set-bundle
	cd desktop && npm run tauri:build

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build / build-cli / build-desktop   Build desktop + unified CLI (or each part)"
	@echo "  build-mobile-android / -ios         Build & install the mobile app (Expo)"
	@echo "  test                                All unit tests (Rust crates + desktop + mobile)"
	@echo "  test-<crate>                        test-agent / -channels / -cli / -tui / -desktop(-rust) / -mobile"
	@echo "  test-cli-diff / -tui-diff / -tui-tmux  [manual] TS→Rust migration gates"
	@echo "  lint                                Rust (CI flags) + desktop + mobile lints"
	@echo "  check-desktop / check-mobile        Lint + typecheck + tests without building apps"
	@echo "  fmt / fmt-mobile                    Format code"
	@echo "  run-agent / -tui / -cli / -channels / -loop   Run a component (debug build)"
	@echo "  run-desktop / run-mobile-android / run-mobile-ios   Run an app in dev mode"
	@echo "  profile-agent / profile-quick / profile-heap  CPU/heap profiling (PROFILE_SECS=30)"
	@echo "  generate-models                     Fetch model data, regenerate Rust catalog + wiki docs"
	@echo "  generate-proto                      Regenerate wire code (rpc future.proto + channels feishu_ws)"
	@echo "  install / install-cli / install-desktop / install-skills   Install to $(PREFIX)"
	@echo "  setup                               Bootstrap a fresh clone (JS deps + skills + sidecar)"
	@echo "  uninstall                           Remove installed binaries from $(PREFIX)"
	@echo "  package-desktop                     Package desktop bundles"
	@echo "  clean                               Remove build artifacts"
