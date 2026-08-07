.PHONY: version build build-agent build-tui build-tui-rust build-cli build-gui build-gui-dist build-channels build-mobile-android test test-mobile lint lint-agent lint-channels lint-tui lint-cli lint-gui lint-mobile stylelint-gui check-gui check-mobile clean run run-agent run-tui run-cli run-gui run-mobile-android run-channels package-gui install install-nogui uninstall install-agent install-tui install-cli install-gui install-channels install-skills install-loop fmt fmt-mobile generate-models generate-proto help test-gui-rust gui-sidecars node-workspace

# ─── Version ──────────────────────────────────────────────────────────────────
# Single source of truth for the build version (see scripts/version.mjs).
# Exported so cargo build.rs and the TS build scripts pick it up. On a release
# tag CI sets FUTURE_VERSION in the environment, which wins over this default.
# Resolve FUTURE_VERSION from git; fall back to 0.0.0-dev if git or the
# version script is unavailable (e.g. Windows without bash).
FUTURE_VERSION_SCRIPT := $(CURDIR)/scripts/version.mjs
export FUTURE_VERSION ?= $(shell node "$(FUTURE_VERSION_SCRIPT)" || node -e "console.log('0.0.0-dev')" || echo 0.0.0-dev)

version:
	@node scripts/version.mjs --json

# ─── Platform ────────────────────────────────────────────────────────────────

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

# ─── Install ──────────────────────────────────────────────────────────────────

install: install-agent install-tui install-cli install-gui install-channels install-skills install-loop

install-nogui: install-agent install-tui install-cli install-channels install-skills install-loop

uninstall:
ifeq ($(OS),windows)
	cmd /c del /q "$(PREFIX)\future-agent$(EXE_SUFFIX)" 2>NUL
	cmd /c del /q "$(PREFIX)\future$(EXE_SUFFIX)" 2>NUL
	cmd /c del /q "$(PREFIX)\future-tui$(EXE_SUFFIX)" 2>NUL
	cmd /c del /q "$(PREFIX)\future-gui$(EXE_SUFFIX)" 2>NUL
	cmd /c del /q "$(PREFIX)\future-channel$(EXE_SUFFIX)" 2>NUL
else
	$(SUDO) rm -f $(PREFIX)/future-agent$(EXE_SUFFIX) $(PREFIX)/future$(EXE_SUFFIX) $(PREFIX)/future-tui$(EXE_SUFFIX) $(PREFIX)/future-gui$(EXE_SUFFIX) $(PREFIX)/future-channel$(EXE_SUFFIX)
endif
	@echo "Removed: future-agent, future, future-tui, future-gui, future-channel"

install-agent: build-agent
ifeq ($(OS),windows)
	$(SUDO) $(COPY_CMD) target\release\future-agent$(EXE_SUFFIX) "$(PREFIX)\future-agent$(EXE_SUFFIX)"
else
	$(SUDO) cp target/release/future-agent "$(PREFIX)/future-agent"
endif

install-tui: build-tui
ifeq ($(OS),windows)
	$(SUDO) $(COPY_CMD) tui\dist\future-tui$(EXE_SUFFIX) "$(PREFIX)\future-tui$(EXE_SUFFIX)"
else
	$(SUDO) cp tui/dist/future-tui "$(PREFIX)/future-tui"
endif

install-cli: build-cli
ifeq ($(OS),windows)
	$(SUDO) $(COPY_CMD) target\release\future$(EXE_SUFFIX) "$(PREFIX)\future$(EXE_SUFFIX)"
else
	$(SUDO) cp target/release/future "$(PREFIX)/future"
endif

install-gui: install-cli install-agent gui-sidecars
	$(call npm-install-if-needed,gui)
	cd gui && npx tauri build --no-bundle
ifeq ($(OS),windows)
	$(SUDO) $(COPY_CMD) gui\src-tauri\target\release\futureos$(EXE_SUFFIX) "$(PREFIX)\future-gui$(EXE_SUFFIX)"
else
	$(SUDO) cp gui/src-tauri/target/release/futureos "$(PREFIX)/future-gui"
endif

install-channels: build-channels
ifeq ($(OS),windows)
	$(SUDO) $(COPY_CMD) target\release\future-channel$(EXE_SUFFIX) "$(PREFIX)\"
else
	$(SUDO) cp target/release/future-channel "$(PREFIX)/"
endif

# Symlink the built-in skill bundles into the agent's app-skills directory
# so the agent discovers them on startup.  Pulls the latest from the skills
# submodule first, then links each skill.  Orphaned symlinks (skills removed
# from the repo) are cleaned up.
install-skills:
	git submodule update --init --remote skills
ifeq ($(OS),windows)
	@if not exist "$(USERPROFILE)\.future\agent\skills" mkdir "$(USERPROFILE)\.future\agent\skills"
	@for /d %%d in (skills\builtin\*) do @( \
		rmdir /s /q "$(USERPROFILE)\.future\agent\skills\%%~nxd" 2>NUL & \
		xcopy /e /i /y "%%d" "$(USERPROFILE)\.future\agent\skills\%%~nxd" >NUL & \
		echo   ✓ %%~nxd \
	)
	@echo Copied built-in skills to ~/.future/agent/skills/
else
	@mkdir -p "$${HOME}/.future/agent/skills"
	@for skill_dir in skills/builtin/*/; do \
		name=$$(basename "$$skill_dir"); \
		link="$${HOME}/.future/agent/skills/$$name"; \
		abs=$$(cd "$$skill_dir" && pwd); \
		rm -rf "$$link"; \
		ln -s "$$abs" "$$link"; \
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
	@echo "Linked built-in skills to ~/.future/agent/skills/"
endif

install-loop:
	bash scripts/install-future-loop.sh $(if $(RELEASE),--release,)

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-agent build-tui build-cli build-gui build-channels

# Only run npm install when package.json is newer than node_modules.
# npm-install-if-needed ─────────────────────────────────────────────────────
# On Unix: only install when package.json is newer than the install stamp.
# On Windows (cmd.exe): skip the bash-conditional (npm install is idempotent).
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

# npm workspace (future-rpc/ts + tui + cli): deps hoist to the repo-root
# node_modules and a single root package-lock.json. Installs only when the
# manifest/lockfile is newer than the install stamp, then builds the shared
# wire-contract package so tui/cli can compile against its dist output.
# (gui and mobile are not workspace members — they keep npm-install-if-needed.)
ifeq ($(OS),windows)
node-workspace:
	@npm install --silent
	@cd future-rpc/ts && npm run build --silent
else
node-workspace:
	@if [ ! -f "node_modules/.package-lock.json" ] || [ "package.json" -nt "node_modules/.package-lock.json" ] || [ "package-lock.json" -nt "node_modules/.package-lock.json" ]; then \
		echo "  npm install (workspace)"; \
		npm install; \
	fi
	@if [ ! -f "future-rpc/ts/dist/index.js" ] || [ -n "$$(find future-rpc/ts/src -name '*.ts' -newer future-rpc/ts/dist/index.js 2>/dev/null)" ]; then \
		echo "  build future-rpc/ts"; \
		cd future-rpc/ts && npm run build; \
	fi
endif

build-agent:
	cd agent && cargo build --release

build-tui: node-workspace
	cd tui && npm run gen-version && npm run build && bun build --compile dist/index.js --outfile dist/future-tui

build-tui-rust:
	rustup run 1.97.0 cargo build -p tui-rust

build-cli:
	cd cli && cargo build --release

# Internal: copy sidecar binaries (agent + CLI) into the Tauri resource dir.
# Tauri's externalBin references these at build time; they are embedded next
# to the GUI binary and extracted on first launch so the GUI can auto-start
# the agent, run CLI commands (skill bootstrap), etc.
gui-sidecars: build-agent build-cli
ifeq ($(OS),windows)
	cmd /c "if not exist gui\src-tauri\binaries mkdir gui\src-tauri\binaries"
	$(COPY_CMD) target\release\future-agent$(EXE_SUFFIX) "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)"
	$(COPY_CMD) target\release\future$(EXE_SUFFIX) "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)"
else
	@mkdir -p gui/src-tauri/binaries
	cp target/release/future-agent gui/src-tauri/binaries/future-agent-$(TARGET)
	cp target/release/future gui/src-tauri/binaries/future-$(TARGET)
endif

# Compile the React frontend only — needed by check-gui and as a dep of build-gui.
build-gui-dist:
	$(call npm-install-if-needed,gui)
	cd gui && npm run build

# Self-contained standalone binary (no installer).  Produces
#   gui/src-tauri/target/release/futureos$(EXE_SUFFIX)
build-gui: build-gui-dist gui-sidecars
	cd gui && npx tauri build --no-bundle

build-channels:
	cd channels && cargo build --release

# Mobile native projects are generated locally by Expo and are intentionally
# not part of the default build. This target builds and installs Android.
build-mobile-android:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run android

# iOS native projects are generated locally by Expo (mobile/ios is gitignored).
# This target prebuilds and launches the app on the iOS simulator.
build-mobile-ios:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run ios

# ─── Test ───────────────────────────────────────────────────────────────────

test: test-agent test-channels test-cli test-cli-rust test-tui test-tui-rust test-tui-diff test-tui-tmux test-gui test-gui-rust test-mobile

test-agent:
	cd agent && cargo test

test-channels:
	cd channels && cargo test

test-cli: test-cli-rust

test-cli-rust:
	rustup run 1.97.0 cargo test -p cli-rust

test-cli-diff:
	./cli/tests/diff-ts-rust.sh

test-tui: node-workspace
	cd tui && npm test

test-tui-rust:
	rustup run 1.97.0 cargo test -p tui-rust

test-tui-diff:
	tui/rust/tests/diff-ts-rust.sh

test-tui-tmux:
	tui/rust/tests/tmux-diff.sh

test-gui:
	$(call npm-install-if-needed,gui)
	cd gui && npm test

test-gui-rust:
	cd gui/src-tauri && cargo test

test-mobile:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm test

# ─── Lint ───────────────────────────────────────────────────────────────────

lint: lint-agent lint-channels lint-tui lint-tui-rust lint-cli lint-gui stylelint-gui lint-mobile

lint-agent:
	cd agent && cargo fmt --check && cargo clippy

lint-channels:
	cd channels && cargo fmt --check && cargo clippy

lint-tui: node-workspace
	cd tui && npm run gen-version && npx tsc --noEmit
	rustup run 1.97.0 cargo clippy -p tui-rust --all-targets -- -D warnings

lint-tui-rust:
	rustup run 1.97.0 cargo clippy -p tui-rust --all-targets -- -D warnings

lint-cli:
	cd cli && rustup run 1.97.0 cargo fmt --check
	rustup run 1.97.0 cargo clippy -p cli-rust --all-targets -- -D warnings

lint-gui:
	cd gui && npm run lint

stylelint-gui:
	cd gui && npm run stylelint

lint-mobile:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run typecheck && npm run lint

check-gui: lint-gui stylelint-gui build-gui-dist
	cd gui/src-tauri && cargo check

check-mobile: lint-mobile test-mobile
	cd mobile && npm run format:check

fmt:
	cd agent && cargo fmt
	cd channels && cargo fmt
	$(MAKE) fmt-mobile

fmt-mobile:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run format

# ─── Run ────────────────────────────────────────────────────────────────────

# Bare --log-file (no value) enables file logging at the default location,
# ~/.future/agent/logs/agent.log; console output stays on the terminal.
run-agent:
	cd agent && cargo run -- --verbose --log-file

run-tui: node-workspace
	cd tui && npm run gen-version && npm run dev

run-cli:
	cd cli && cargo run

run-gui: build-gui
ifeq ($(OS),windows)
	@if not exist gui\src-tauri\binaries mkdir gui\src-tauri\binaries
	@if not exist "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)" "$(MAKE)" build-agent
	@if not exist "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)" $(COPY_CMD) target\release\future-agent$(EXE_SUFFIX) "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)"
	@if not exist "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)" "$(MAKE)" build-cli
	@if not exist "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)" $(COPY_CMD) target\release\future$(EXE_SUFFIX) "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)"
	cd gui && npm run tauri:dev
else
	@mkdir -p gui/src-tauri/binaries
	@if [ ! -f "gui/src-tauri/binaries/future-agent-$(TARGET)" ]; then \
		$(MAKE) build-agent && \
		cp target/release/future-agent "gui/src-tauri/binaries/future-agent-$(TARGET)"; \
	fi
	@if [ ! -f "gui/src-tauri/binaries/future-$(TARGET)" ]; then \
		$(MAKE) build-cli && \
		cp target/release/future "gui/src-tauri/binaries/future-$(TARGET)"; \
	fi
	cd gui && npm run tauri:dev
endif

run-mobile-android:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run android:device

run-mobile-ios:
	$(call npm-install-if-needed,mobile)
	cd mobile && npm run ios

package-gui: install-gui
	node scripts/version.mjs --set-bundle
	cd gui && npm run tauri:build

run-channels:
	cd channels && cargo run

# ─── Profile ───────────────────────────────────────────────────────────────

# Build agent with debug symbols + frame pointers for profiling, then run
# the benchmark suite.  Writes flamegraph SVG + logs to profile-results/.
profile-agent:
ifeq ($(OS),windows)
	set "RUSTFLAGS=-C force-frame-pointers=yes" && \
		cargo build --release -p future-agent \
		--config "profile.release.debug=""line-tables-only""" \
		--config "profile.release.strip=""none"""
	@if not exist profile-results mkdir profile-results
	@where blondie >NUL 2>NUL || (echo. & echo  blondie is required for CPU profiling on Windows. & echo  Install: cargo install blondie --features inferno & echo  CPU profiling also requires administrator privileges. & exit /b 1)
	@echo Starting profile run (port 50052, 90s)...
	set "PROFILE_DURATION=90" && powershell -ExecutionPolicy Bypass -File scripts/agent-profile-bench.ps1
else
	RUSTFLAGS="-C force-frame-pointers=yes" \
		cargo build --release -p future-agent \
		--config 'profile.release.debug="line-tables-only"' \
		--config 'profile.release.strip="none"'
	@mkdir -p profile-results
	@echo "Starting profile run (port 50052, 90s)..."
	PROFILE_DURATION=90 bash scripts/agent-profile-bench.sh
	@echo ""
	@echo "Flamegraph: $$(ls -t profile-results/agent-profile-*.svg | head -1)"
	@echo "Run: open profile-results/agent-profile-*.svg"
endif

# Heap (memory) profile: build with the dhat-heap feature, run on port
# 50052 for N seconds, write a dhat report JSON.
# Usage: make profile-heap PROFILE_SECS=30
# View the report at https://nnethercote.github.io/dh_view/dh_view.html
profile-heap:
ifeq ($(OS),windows)
# Windows needs full debug info (2) for dhat backtrace capture (line-tables-only is insufficient)
	cargo build --release -p future-agent --features dhat-heap \
		--config profile.release.debug=2 \
		--config "profile.release.strip=""none"""
	@if not exist profile-results mkdir profile-results
else
	cargo build --release -p future-agent --features dhat-heap \
		--config 'profile.release.debug="line-tables-only"' \
		--config 'profile.release.strip="none"'
	@mkdir -p profile-results
endif
	./target/release/future-agent$(EXE_SUFFIX) \
		--grpc-addr 127.0.0.1:50052 \
		--profile-heap profile-results/heap-profile.json \
		--profile-seconds $(or $(PROFILE_SECS),30) \
		--verbose
	@echo ""
	@echo "Heap profile: profile-results/heap-profile.json"
	@echo "Interactive viewer: https://nnethercote.github.io/dh_view/dh_view.html"
	@echo "For static flamegraph: dhat-to-flamegraph profile-results/heap-profile.json -f svg -o heap-flame.svg (requires dhat backtraces)"

# Quick profile: start agent with profiling on port 50052, run for N seconds.
# Usage: make profile-quick PROFILE_SECS=30
profile-quick:
ifeq ($(OS),windows)
	set "RUSTFLAGS=-C force-frame-pointers=yes" && \
		cargo build --release -p future-agent \
		--config "profile.release.debug=""line-tables-only""" \
		--config "profile.release.strip=""none"""
	powershell -ExecutionPolicy Bypass -File scripts/profile-quick.ps1 -Duration $(or $(PROFILE_SECS),30)
else
	RUSTFLAGS="-C force-frame-pointers=yes" \
		cargo build --release -p future-agent \
		--config 'profile.release.debug="line-tables-only"' \
		--config 'profile.release.strip="none"'
	@mkdir -p profile-results
	./target/release/future-agent \
		--grpc-addr 127.0.0.1:50052 \
		--profile profile-results/quick-profile.svg \
		--profile-seconds $(or $(PROFILE_SECS),30) \
		--verbose
endif

# ─── Generate ───────────────────────────────────────────────────────────────

generate-models:
	python3 scripts/generate_models.py

generate-proto:
	cd future-rpc/rust && REGENERATE_PROTO=1 cargo build
	cd channels && REGENERATE_PROTO=1 cargo build
	cd future-rpc/ts && bun run scripts/generate-proto.ts

# ─── Clean ──────────────────────────────────────────────────────────────────

clean:
ifeq ($(OS),windows)
	@if exist target rmdir /s /q target
	@if exist node_modules rmdir /s /q node_modules
	@if exist future-rpc\ts\dist rmdir /s /q future-rpc\ts\dist
	@if exist tui\dist rmdir /s /q tui\dist
	@if exist tui\node_modules rmdir /s /q tui\node_modules
	@if exist tui\future-tui del /q tui\future-tui
	@if exist tui\src\version.generated.ts del /q tui\src\version.generated.ts
	@if exist cli\dist rmdir /s /q cli\dist
	@if exist cli\node_modules rmdir /s /q cli\node_modules
	@if exist cli\src\version.generated.ts del /q cli\src\version.generated.ts
	@if exist gui\dist rmdir /s /q gui\dist
	@if exist gui\node_modules rmdir /s /q gui\node_modules
	@if exist gui\src-tauri\target rmdir /s /q gui\src-tauri\target
	@if exist gui\src-tauri\binaries rmdir /s /q gui\src-tauri\binaries
	@if exist "$(PREFIX)\future-agent$(EXE_SUFFIX)" del /q "$(PREFIX)\future-agent$(EXE_SUFFIX)"
	@if exist "$(PREFIX)\future$(EXE_SUFFIX)" del /q "$(PREFIX)\future$(EXE_SUFFIX)"
	@if exist "$(PREFIX)\future-tui$(EXE_SUFFIX)" del /q "$(PREFIX)\future-tui$(EXE_SUFFIX)"
	@if exist "$(PREFIX)\future-gui$(EXE_SUFFIX)" del /q "$(PREFIX)\future-gui$(EXE_SUFFIX)"
	@if exist "$(PREFIX)\future-channel$(EXE_SUFFIX)" del /q "$(PREFIX)\future-channel$(EXE_SUFFIX)"
else
	rm -rf target
	rm -rf node_modules future-rpc/ts/dist
	rm -rf tui/dist tui/node_modules
	rm -f tui/future-tui tui/src/version.generated.ts
	rm -rf gui/dist gui/node_modules gui/src-tauri/target gui/src-tauri/binaries
	$(SUDO) rm -f $(PREFIX)/future-agent$(EXE_SUFFIX) $(PREFIX)/future$(EXE_SUFFIX) $(PREFIX)/future-tui$(EXE_SUFFIX) $(PREFIX)/future-gui$(EXE_SUFFIX) $(PREFIX)/future-channel$(EXE_SUFFIX)
endif

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build              Build agent, TUI, CLI, and GUI"
	@echo "  build-agent        Build Rust agent"
	@echo "  build-tui          Build standalone TUI binary"
	@echo "  build-tui-rust     Build the Rust TUI port (future-tui + examples)"
	@echo "  build-cli          Build TypeScript CLI"
	@echo "  build-gui          Build React/Tauri GUI frontend"
	@echo "  build-channels      Build channel bridge"
	@echo "  build-mobile-android Generate, build, and install the Android app"
	@echo "  build-mobile-ios     Generate, build, and install the iOS app (requires Xcode)"
	@echo "  check-mobile       Typecheck, lint, format-check, and test mobile"
	@echo "  test               Run all tests (Rust crates + cli/tui/gui/mobile)"
	@echo "  test-tui-rust      Run the Rust TUI port unit tests (cargo test -p tui-rust)"
	@echo "  test-tui-diff      TS vs Rust render parity (byte-compare tui/rust/tests/diff-ts-rust.sh)"
	@echo "  test-tui-tmux      TS vs Rust tmux screen consistency + goldens (tui/rust/tests/tmux-diff.sh)"
	@echo "  test-cli-rust      Run the Rust CLI port unit tests (cargo test -p cli-rust)"
	@echo "  test-cli-diff      Differential test: TS future vs Rust future, byte-identical output"
	@echo "  lint               Lint all (agent + channels + TUI + CLI + GUI + mobile)"
	@echo "  fmt                Format Rust and mobile code"
	@echo "  run-agent          Run agent directly (debug build)"
	@echo "  run-tui            Run TUI in dev mode"
	@echo "  run-cli            Run CLI in dev mode"
	@echo "  run-gui            Run GUI in dev mode"
	@echo "  run-mobile-android Run the Android app on a selected device"
	@echo "  run-mobile-ios     Run the iOS app on the simulator (requires Xcode)"
	@echo "  run-channels        Run channel bridge directly (debug build)"
	@echo "  package-gui        Package GUI desktop bundles"
	@echo "  profile-agent      CPU profile: build + 90s bench, write flamegraph SVG"
	@echo "  profile-quick      CPU profile: run agent N secs (PROFILE_SECS=30)"
	@echo "  profile-heap       Heap profile via dhat, write dhat report JSON"
	@echo "  generate-models    Fetch model data, regenerate Rust catalog + wiki docs"
	@echo "  generate-proto     Regenerate wire code: future-rpc (future.proto, all TS clients) + channels feishu_ws"
	@echo "  install            Build & install all components"
	@echo "  install-nogui      Build & install terminal stack (skip GUI)"
	@echo "  uninstall          Remove installed binaries from $(PREFIX)/"
	@echo "  clean              Remove build artifacts + installed binaries"
