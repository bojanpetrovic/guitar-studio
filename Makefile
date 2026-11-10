# Guitar Studio — project tasks. Run `make` or `make help` to list them.

SHELL        := /bin/bash
APP_DATA     := $(HOME)/Library/Application Support/dev.guitarstudio.app
DEBUG_BIN    := target/debug/guitar-studio
FORMAT       ?= au
PLUGIN       ?= nolly

.DEFAULT_GOAL := help
.PHONY: help install dev build bundle run test test-engine lint fmt fmt-check check \
        smoke preset-test param-test params scan icons clean clean-all reset-settings

help: ## Show this help
	@awk 'BEGIN {FS = ":.*## "; printf "\nUsage: make \033[36m<target>\033[0m [VAR=value]\n\n"} \
		/^[a-zA-Z_-]+:.*## / { printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@printf "\nVariables: FORMAT=$(FORMAT) PLUGIN=$(PLUGIN) (used by smoke, preset-test, param-test, params)\n\n"

# ---------------------------------------------------------------- setup

install: node_modules ## Install JS dependencies and fetch Rust crates
	cargo fetch

node_modules: package.json
	npm install
	@touch node_modules

# ---------------------------------------------------------------- develop

dev: node_modules ## Run the app with hot reload
	npm run tauri dev

run: build ## Run the release binary (no bundle)
	./target/release/guitar-studio

# ---------------------------------------------------------------- build

build: node_modules ## Build frontend + release binary
	npm run build
	cargo build --release -p guitar-studio

bundle: node_modules ## Build the macOS .app and .dmg
	npm run tauri build

icons: node_modules ## Regenerate app icons from assets/app-icon.svg
	npx tauri icon assets/app-icon.svg -o src-tauri/icons
	rm -rf src-tauri/icons/android src-tauri/icons/ios src-tauri/icons/Square* src-tauri/icons/StoreLogo.png

# ---------------------------------------------------------------- quality

test: test-engine ## Run all tests
	npx tsc --noEmit

test-engine: ## Run Rust engine unit tests
	cargo test -p gs-engine

lint: node_modules ## Clippy + TypeScript type-check
	cargo clippy -p gs-engine -p guitar-studio --all-targets
	npx tsc --noEmit

fmt: ## Format Rust code
	cargo fmt --all

fmt-check: ## Check Rust formatting (CI)
	cargo fmt --all -- --check

check: fmt-check lint test ## Everything CI should run

# ---------------------------------------------------------------- plugins

smoke: ## Load + process a plugin headless, e.g. make smoke FORMAT=vst3 PLUGIN=neural
	cargo run -p gs-engine --example plugin_smoke -- $(FORMAT) "$(PLUGIN)"

preset-test: ## Save + reload a rig with a real plugin, e.g. make preset-test FORMAT=au PLUGIN=nolly
	cargo run -p gs-engine --example preset_roundtrip -- $(FORMAT) "$(PLUGIN)"

param-test: ## Set + read back a plugin's parameters, e.g. make param-test FORMAT=au PLUGIN=nolly
	cargo run -p gs-engine --example param_control -- $(FORMAT) "$(PLUGIN)"

params: ## List a plugin's parameters and ids, e.g. make params FORMAT=au PLUGIN=nolly
	cargo run -q -p gs-engine --example list_params -- $(FORMAT) "$(PLUGIN)"

scan: ## Rescan all plugins (crash-isolated) into the app's cache
	cargo build -p guitar-studio
	@mkdir -p "$(APP_DATA)"
	cargo run -q -p gs-engine --example scan_catalog -- "$(CURDIR)/$(DEBUG_BIN)" "$(APP_DATA)/plugins.json"

# ---------------------------------------------------------------- housekeeping

reset-settings: ## Forget saved audio device settings and plugin cache
	rm -f "$(APP_DATA)/audio.json" "$(APP_DATA)/plugins.json"

clean: ## Remove build outputs
	cargo clean
	rm -rf dist

clean-all: clean ## Also remove node_modules
	rm -rf node_modules
