.DEFAULT_GOAL := help

# ── Build ──────────────────────────────────────────────────────────────────────

.PHONY: build
build: ## Compile the full workspace
	cargo build --workspace

# ── Test ───────────────────────────────────────────────────────────────────────

.PHONY: test
test: ## Run lib tests across all crates except cairn-app (fast)
	cargo test --workspace --exclude cairn-app --lib

.PHONY: test-all
test-all: ## Run every test in the workspace (lib + integration)
	cargo test --workspace

.PHONY: pg-test
pg-test: ## Run cairn-store tests with postgres + sqlite features enabled
	cargo test -p cairn-store --features postgres,sqlite

# ── Run ────────────────────────────────────────────────────────────────────────

.PHONY: server
server: ## Start the cairn-app server (local mode, 127.0.0.1:3000)
	cargo run -p cairn-app

# ── Docker ─────────────────────────────────────────────────────────────────────

.PHONY: docker
docker: ## Build and start all services via docker compose
	docker compose up --build

# ── Maintenance ────────────────────────────────────────────────────────────────

.PHONY: clean
clean: ## Remove all build artifacts
	cargo clean

.PHONY: fmt
fmt: ## Format all Rust source files
	cargo fmt

.PHONY: clippy
clippy: ## Run Clippy lints across the full workspace
	cargo clippy --workspace

# ── Help ───────────────────────────────────────────────────────────────────────

.PHONY: help
help: ## Show this help message
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*##"}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'
