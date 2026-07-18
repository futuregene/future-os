.PHONY: version build build-agent build-tui build-cli build-gui build-channels test lint lint-agent lint-channels lint-tui lint-cli lint-gui stylelint-gui check-gui clean run run-agent run-tui run-cli run-gui run-channels package-gui install install-nogui uninstall install-agent install-tui install-cli install-gui install-channels install-skills fmt generate-models generate-proto help

# ─── Version ──────────────────────────────────────────────────────────────────
# Single source of truth for the build version (see scripts/version.mjs).
# Exported so cargo build.rs and the TS build scripts pick it up. On a release
# tag CI sets FUTURE_VERSION in the environment, which wins over this default.
# Resolve FUTURE_VERSION from git; fall back to 0.0.0-dev if git or the
# version script is unavailable (e.g. Windows without bash).
FUTURE_VERSION_SCRIPT := $(CURDIR)/scripts/version.mjs
export FUTURE_VERSION ?= $(shell node "$(FUTURE_VERSION_SCRIPT)" 2>NUL || node -e "console.log('0.0.0-dev')" 2>NUL || echo 0.0.0-dev)

version:
	@node scripts/version.mjs --json

# ─── Platform ────────────────────────────────────────────────────────────────

TARGET := $(shell rustc -vV | node -e "process.stdin.on('data',d=>{const m=d.toString().match(/host:\s*(.+)/);if(m)console.log(m[1])})")
OS := $(word 3,$(subst -, ,$(TARGET)))

ifeq ($(OS),darwin)
  PREFIX := /opt/homebrew/bin
  SUDO :=
else ifeq ($(OS),linux)
  PREFIX := /usr/local/bin
  SUDO := sudo
else
  PREFIX := $(USERPROFILE)/.future/bin
  SUDO :=
endif

# ─── Install ──────────────────────────────────────────────────────────────────

install: install-agent install-tui install-cli install-gui install-channels install-skills

install-nogui: install-agent install-tui install-cli install-channels install-skills

uninstall:
	$(SUDO) rm -f $(PREFIX)/future-agent $(PREFIX)/future $(PREFIX)/future-tui $(PREFIX)/future-gui $(PREFIX)/future-channel
	@echo "Removed: future-agent, future, future-tui, future-gui, future-channel"

install-agent: build-agent
	$(SUDO) cp agent/target/release/future-agent $(PREFIX)/future-agent

install-tui: build-tui
	$(SUDO) cp tui/dist/future-tui $(PREFIX)/future-tui

install-cli: build-cli
	$(SUDO) cp cli/dist/future $(PREFIX)/future

install-gui: install-cli install-agent
	@mkdir -p gui/src-tauri/binaries
	cp agent/target/release/future-agent gui/src-tauri/binaries/future-agent-$(TARGET)
	cp cli/dist/future gui/src-tauri/binaries/future-$(TARGET)
	$(call npm-install-if-needed,gui)
	cd gui && npx tauri build --no-bundle
	$(SUDO) cp gui/src-tauri/target/release/futureos $(PREFIX)/future-gui

install-channels: build-channels
	$(SUDO) cp channels/target/release/future-channel $(PREFIX)/

# Symlink the built-in skill bundles into the agent's app-skills directory
# so the agent discovers them on startup.  Pulls the latest from the skills
# submodule first, then links each skill.  Orphaned symlinks (skills removed
# from the repo) are cleaned up.
install-skills:
	git submodule update --init --remote skills
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

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-agent build-tui build-cli build-gui build-channels

# Only run npm install when package.json is newer than node_modules.
define npm-install-if-needed
	@if [ ! -f "$(1)/node_modules/.package-lock.json" ] || [ "$(1)/package.json" -nt "$(1)/node_modules/.package-lock.json" ]; then \
		echo "  npm install $(1)/"; \
		cd $(1) && npm install; \
	fi
endef

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

test:
	cd agent && cargo test

# ─── Lint ───────────────────────────────────────────────────────────────────

lint: lint-agent lint-channels lint-tui lint-cli lint-gui stylelint-gui

lint-agent:
	cd agent && cargo fmt --check && cargo clippy

lint-channels:
	cd channels && cargo fmt --check && cargo clippy

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

run-agent:
	cd agent && cargo run -- --verbose

run-tui:
	$(call npm-install-if-needed,tui)
	cd tui && npm run gen-version && npm run dev

run-cli:
	$(call npm-install-if-needed,cli)
	cd cli && npm run gen-version && npm run dev

run-gui: build-gui
	@mkdir -p gui/src-tauri/binaries
	@if [ ! -f "gui/src-tauri/binaries/future-agent-$(TARGET)" ]; then \
		$(MAKE) build-agent && \
		cp agent/target/release/future-agent "gui/src-tauri/binaries/future-agent-$(TARGET)"; \
	fi
	@if [ ! -f "gui/src-tauri/binaries/future-$(TARGET)" ]; then \
		cd cli && npm install && npm run build && \
		bun build --compile dist/index.js --outfile dist/future && \
		cd .. && cp cli/dist/future "gui/src-tauri/binaries/future-$(TARGET)"; \
	fi
	cd gui && npm run tauri:dev

package-gui: install-gui
	node scripts/version.mjs --set-bundle
	cd gui && npm run tauri:build

run-channels:
	cd channels && cargo run

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
	rm -rf agent/target
	rm -rf channels/target
	rm -rf remote/target
	rm -rf tui/dist tui/node_modules
	rm -f tui/future-tui tui/src/version.generated.ts
	rm -rf cli/dist cli/node_modules
	rm -f cli/src/version.generated.ts
	rm -rf gui/dist gui/node_modules gui/src-tauri/target gui/src-tauri/binaries
	$(SUDO) rm -f $(PREFIX)/future-agent $(PREFIX)/future $(PREFIX)/future-tui $(PREFIX)/future-gui $(PREFIX)/future-channel

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build              Build agent, TUI, CLI, and GUI"
	@echo "  build-agent        Build Rust agent"
	@echo "  build-tui          Build standalone TUI binary"
	@echo "  build-cli          Build TypeScript CLI"
	@echo "  build-gui          Build React/Tauri GUI frontend"
	@echo "  build-channels      Build channel bridge"
	@echo "  test               Run Rust tests (agent)"
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
