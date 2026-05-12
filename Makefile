.PHONY: build build-cli build-web test test-verbose clean lint run run-web install

# Binary names
CLI_BIN   = xihu
WEB_BIN   = xihu-web

# Go module
MODULE    = github.com/huichen/xihu
GO        = go
GOFLAGS   = -mod=mod

# Build output
BUILD_DIR = bin

# Default target
all: build

# ─── Build ──────────────────────────────────────────────────────────────────

## Build both binaries
build: build-cli build-web

## Build CLI binary
build-cli:
	@mkdir -p $(BUILD_DIR)
	CGO_ENABLED=0 $(GO) build $(GOFLAGS) -o $(BUILD_DIR)/$(CLI_BIN) ./cmd/xihu/

## Build web binary
build-web:
	@mkdir -p $(BUILD_DIR)
	CGO_ENABLED=0 $(GO) build $(GOFLAGS) -o $(BUILD_DIR)/$(WEB_BIN) ./cmd/xihu-web/

# ─── Test ───────────────────────────────────────────────────────────────────

## Run all tests
test:
	$(GO) test $(GOFLAGS) -count=1 -timeout 120s ./...

## Run tests with verbose output
test-verbose:
	$(GO) test $(GOFLAGS) -count=1 -timeout 120s -v ./...

## Run tests with race detector
test-race:
	$(GO) test $(GOFLAGS) -count=1 -timeout 120s -race ./...

## Run tests with coverage
test-cover:
	$(GO) test $(GOFLAGS) -count=1 -timeout 120s -coverprofile=coverage.out ./...
	$(GO) tool cover -func=coverage.out

## Show coverage in browser
test-cover-html:
	$(GO) test $(GOFLAGS) -count=1 -timeout 120s -coverprofile=coverage.out ./...
	$(GO) tool cover -html=coverage.out

# ─── Lint ───────────────────────────────────────────────────────────────────

## Run go vet
lint:
	$(GO) vet $(GOFLAGS) ./...

## Run go fmt check
fmt-check:
	@test -z "$$($(GO) fmt ./...)" || (echo "Files need formatting. Run 'make fmt'." && exit 1)

## Run go fmt
fmt:
	$(GO) fmt ./...

# ─── Run ────────────────────────────────────────────────────────────────────

## Build and run CLI (pass ARGS="--help" for flags)
run: build-cli
	./$(BUILD_DIR)/$(CLI_BIN) $(ARGS)

## Build and run web server (pass PORT=9090 for custom port)
run-web: build-web
	PORT=$(or $(PORT),8080) ./$(BUILD_DIR)/$(WEB_BIN)

# ─── Install ────────────────────────────────────────────────────────────────

## Install binaries to GOPATH/bin or /usr/local/bin
install: build
	$(GO) install $(GOFLAGS) ./cmd/xihu/
	$(GO) install $(GOFLAGS) ./cmd/xihu-web/

# ─── Clean ──────────────────────────────────────────────────────────────────

## Clean build artifacts
clean:
	rm -rf $(BUILD_DIR)
	rm -f coverage.out

## Clean and remove Go module cache
clean-all: clean
	$(GO) clean -cache -modcache -testcache

# ─── Help ───────────────────────────────────────────────────────────────────

## Show this help
help:
	@grep -E '^##|^[a-zA-Z_-]+:' Makefile | \
		grep -B1 '^##' | \
		sed -n 's/^## //p' | \
		awk 'NR%2{printf "  \033[36m%-20s\033[0m", $$0; next} {print "- "$$0}'

# ─── Code Generation ────────────────────────────────────────────────────────

## Generate built-in model catalog from external APIs (matches pi's generate-models.ts)
generate-models:
	cd internal/modelregistry && $(GO) run generate_models.go
