.DEFAULT_GOAL := help

# ── Config ────────────────────────────────────────────────────────────────────

CAIRN_ADDR  ?= 0.0.0.0
CAIRN_PORT  ?= 3000
CAIRN_TOKEN ?= cairn-demo-token
OLLAMA_HOST ?= http://localhost:11434

# ── Composite ─────────────────────────────────────────────────────────────────

.PHONY: all
all: install check test ui-build build ## Install deps, check, test, build everything

# ── Development server ────────────────────────────────────────────────────────

.PHONY: dev
dev: ## Run cairn-app dev server (cargo run, 0.0.0.0:3000)
	CAIRN_ADMIN_TOKEN=$(CAIRN_TOKEN) OLLAMA_HOST=$(OLLAMA_HOST) \
	cargo run -p cairn-app -- --addr $(CAIRN_ADDR) --port $(CAIRN_PORT)

# ── Build ─────────────────────────────────────────────────────────────────────

.PHONY: build
build: ## Build cairn-app release binary
	cargo build --release -p cairn-app

# ── Test ──────────────────────────────────────────────────────────────────────

.PHONY: test
test: ## Run the full workspace test suite
	cargo test --workspace

.PHONY: test-all
test-all: ## Alias for test (lib + integration)
	cargo test --workspace

.PHONY: pg-test
pg-test: ## cairn-store tests with postgres + sqlite features
	cargo test -p cairn-store --features postgres,sqlite

# ── Type check ────────────────────────────────────────────────────────────────

.PHONY: check
check: ## cargo check + TypeScript type check
	cargo check --workspace
	cd ui && npx tsc --noEmit

# ── Formatting & linting ──────────────────────────────────────────────────────

.PHONY: fmt
fmt: ## Format all Rust source files
	cargo fmt --all

.PHONY: clippy
clippy: ## Run Clippy lints across the full workspace
	cargo clippy --workspace

# ── UI ────────────────────────────────────────────────────────────────────────

.PHONY: install
install: ## Install UI npm dependencies
	cd ui && npm install

.PHONY: ui-dev
ui-dev: ## Start Vite dev server with HMR (http://localhost:5173)
	cd ui && npm run dev

.PHONY: ui-build
ui-build: ## Production UI build (output: ui/dist/)
	cd ui && npm run build

# ── Docker ────────────────────────────────────────────────────────────────────

.PHONY: docker
docker: ## Build and start all services via docker compose
	docker compose up --build

# ── Smoke test ────────────────────────────────────────────────────────────────

.PHONY: smoke
smoke: ## Run API smoke test suite against a running server
	bash scripts/smoke-test.sh

# ── Legacy alias ──────────────────────────────────────────────────────────────

.PHONY: server
server: dev ## Alias for 'dev'

# ── Clean ─────────────────────────────────────────────────────────────────────

.PHONY: clean
clean: ## Remove Rust build artefacts and UI output
	cargo clean
	rm -rf ui/dist ui/node_modules

# ── Help ──────────────────────────────────────────────────────────────────────

.PHONY: help
help: ## Show this help message
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*##"}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'
