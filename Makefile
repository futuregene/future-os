.PHONY: version build build-agent build-tui build-cli build-gui build-channels test lint lint-agent lint-channels lint-tui lint-cli lint-gui stylelint-gui check-gui clean run run-agent run-tui run-cli run-gui run-channels package-gui install install-nogui uninstall install-agent install-tui install-cli install-gui install-channels install-skills fmt generate-models generate-proto help test-gui-rust

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

install: install-agent install-tui install-cli install-gui install-channels install-skills

install-nogui: install-agent install-tui install-cli install-channels install-skills

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
	$(SUDO) $(COPY_CMD) cli\dist\future$(EXE_SUFFIX) "$(PREFIX)\future$(EXE_SUFFIX)"
else
	$(SUDO) cp cli/dist/future "$(PREFIX)/future"
endif

install-gui: install-cli install-agent
ifeq ($(OS),windows)
	cmd /c "if not exist gui\src-tauri\binaries mkdir gui\src-tauri\binaries"
	$(COPY_CMD) target\release\future-agent$(EXE_SUFFIX) "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)"
	$(COPY_CMD) cli\dist\future$(EXE_SUFFIX) "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)"
else
	@mkdir -p gui/src-tauri/binaries
	cp target/release/future-agent gui/src-tauri/binaries/future-agent-$(TARGET)
	cp cli/dist/future gui/src-tauri/binaries/future-$(TARGET)
endif
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

build-agent:
	cd agent && cargo build --release

build-tui:
	$(call npm-install-if-needed,tui)
	cd tui && npm run gen-version && npm run build && bun build --compile dist/index.js --outfile dist/future-tui

build-cli:
	$(call npm-install-if-needed,cli)
	cd cli && npm run gen-version && npm run build && bun build --compile dist/index.js --outfile dist/future

build-gui:
	$(call npm-install-if-needed,gui)
	cd gui && npm run build

build-channels:
	cd channels && cargo build --release

# ─── Test ───────────────────────────────────────────────────────────────────

test: test-agent test-channels test-remote test-cli test-tui test-gui test-gui-rust

test-agent:
	cd agent && cargo test

test-channels:
	cd channels && cargo test

test-remote:
	cd remote && cargo test

test-cli:
	$(call npm-install-if-needed,cli)
	cd cli && npm test

test-tui:
	$(call npm-install-if-needed,tui)
	cd tui && npm test

test-gui:
	$(call npm-install-if-needed,gui)
	cd gui && npm test

test-gui-rust:
	cd gui/src-tauri && cargo test

# ─── Lint ───────────────────────────────────────────────────────────────────

lint: lint-agent lint-channels lint-remote lint-tui lint-cli lint-gui stylelint-gui

lint-agent:
	cd agent && cargo fmt --check && cargo clippy

lint-channels:
	cd channels && cargo fmt --check && cargo clippy

lint-remote:
	cd remote && cargo fmt --check && cargo clippy

lint-tui:
	cd tui && npm run gen-version && npx tsc --noEmit

lint-cli:
	cd cli && npm run gen-version && npx tsc --noEmit

lint-gui:
	cd gui && npm run lint

stylelint-gui:
	cd gui && npm run stylelint

check-gui: lint-gui stylelint-gui build-gui
	cd gui/src-tauri && cargo check

fmt:
	cd agent && cargo fmt
	cd channels && cargo fmt

# ─── Run ────────────────────────────────────────────────────────────────────

# Bare --log-file (no value) enables file logging at the default location,
# ~/.future/agent/logs/agent.log; console output stays on the terminal.
run-agent:
	cd agent && cargo run -- --verbose --log-file

run-tui:
	$(call npm-install-if-needed,tui)
	cd tui && npm run gen-version && npm run dev

run-cli:
	$(call npm-install-if-needed,cli)
	cd cli && npm run gen-version && npm run dev

run-gui: build-gui
ifeq ($(OS),windows)
	@if not exist gui\src-tauri\binaries mkdir gui\src-tauri\binaries
	@if not exist "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)" "$(MAKE)" build-agent
	@if not exist "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)" $(COPY_CMD) target\release\future-agent$(EXE_SUFFIX) "gui\src-tauri\binaries\future-agent-$(TARGET)$(EXE_SUFFIX)"
	@if not exist "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)" "$(MAKE)" build-cli
	@if not exist "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)" $(COPY_CMD) cli\dist\future$(EXE_SUFFIX) "gui\src-tauri\binaries\future-$(TARGET)$(EXE_SUFFIX)"
	cd gui && npm run tauri:dev
else
	@mkdir -p gui/src-tauri/binaries
	@if [ ! -f "gui/src-tauri/binaries/future-agent-$(TARGET)" ]; then \
		$(MAKE) build-agent && \
		cp target/release/future-agent "gui/src-tauri/binaries/future-agent-$(TARGET)"; \
	fi
	@if [ ! -f "gui/src-tauri/binaries/future-$(TARGET)" ]; then \
		cd cli && npm install && npm run build && \
		bun build --compile dist/index.js --outfile dist/future && \
		cd .. && cp cli/dist/future "gui/src-tauri/binaries/future-$(TARGET)"; \
	fi
	cd gui && npm run tauri:dev
endif

package-gui: install-gui
	node scripts/version.mjs --set-bundle
	cd gui && npm run tauri:build

run-channels:
	cd channels && cargo run

# ─── Profile ───────────────────────────────────────────────────────────────

# Build agent with debug symbols + frame pointers for profiling, then run
# the benchmark suite.  Writes flamegraph SVG + logs to profile-results/.
profile-agent:
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

# Quick profile: start agent with profiling on port 50052, run for N seconds.
# Usage: make profile-quick PROFILE_SECS=30
profile-quick:
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

# ─── Generate ───────────────────────────────────────────────────────────────

generate-models:
	python3 scripts/generate_models.py

generate-proto:
	REGENERATE_PROTO=1 cd agent && cargo build
	REGENERATE_PROTO=1 cd channels && cargo build
	REGENERATE_PROTO=1 cd gui/src-tauri && cargo build
	cd tui && npm run generate-proto

# ─── Clean ──────────────────────────────────────────────────────────────────

clean:
ifeq ($(OS),windows)
	@if exist target rmdir /s /q target
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
	rm -rf tui/dist tui/node_modules
	rm -f tui/future-tui tui/src/version.generated.ts
	rm -rf cli/dist cli/node_modules
	rm -f cli/src/version.generated.ts
	rm -rf gui/dist gui/node_modules gui/src-tauri/target gui/src-tauri/binaries
	$(SUDO) rm -f $(PREFIX)/future-agent$(EXE_SUFFIX) $(PREFIX)/future$(EXE_SUFFIX) $(PREFIX)/future-tui$(EXE_SUFFIX) $(PREFIX)/future-gui$(EXE_SUFFIX) $(PREFIX)/future-channel$(EXE_SUFFIX)
endif

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build              Build agent, TUI, CLI, and GUI"
	@echo "  build-agent        Build Rust agent"
	@echo "  build-tui          Build standalone TUI binary"
	@echo "  build-cli          Build TypeScript CLI"
	@echo "  build-gui          Build React/Tauri GUI frontend"
	@echo "  build-channels      Build channel bridge"
	@echo "  test               Run all tests (Rust crates + cli/tui/gui)"
	@echo "  lint               Lint all (agent + channels + TUI + CLI + GUI)"
	@echo "  fmt                Format Rust code (agent + channels)"
	@echo "  run-agent          Run agent directly (debug build)"
	@echo "  run-tui            Run TUI in dev mode"
	@echo "  run-cli            Run CLI in dev mode"
	@echo "  run-gui            Run GUI in dev mode"
	@echo "  run-channels        Run channel bridge directly (debug build)"
	@echo "  package-gui        Package GUI desktop bundles"
	@echo "  generate-models    Fetch model data, regenerate Rust catalog + wiki docs"
	@echo "  generate-proto     Compile proto/future.proto to Rust gRPC code"
	@echo "  install            Build & install all components"
	@echo "  install-nogui      Build & install terminal stack (skip GUI)"
	@echo "  uninstall          Remove installed binaries from $(PREFIX)/"
	@echo "  clean              Remove build artifacts + installed binaries"
