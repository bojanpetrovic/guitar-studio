# Guitar Studio — project tasks. Run `make` or `make help` to list them.

SHELL        := /bin/bash
APP_DATA     := $(HOME)/Library/Application Support/dev.guitarstudio.app
PLUGIN_DIR   := $(HOME)/Library/Audio/Plug-Ins
# Some Command Line Tools installs leave an incomplete C++ header folder that shadows the
# SDK's (the VST3 wrapper compiles C++); point the compiler at the SDK's headers then.
SDK_CXX      := $(shell xcrun --show-sdk-path 2>/dev/null)/usr/include/c++/v1
CXX_FIX      := $(if $(wildcard /Library/Developer/CommandLineTools/usr/include/c++/v1/cstdint),,CXXFLAGS="-nostdinc++ -isystem $(SDK_CXX)")
FORMAT       ?= au
PLUGIN       ?= nolly

.DEFAULT_GOAL := help
.PHONY: help install dev build bundle package docs-screenshots run test test-engine lint fmt fmt-check check \
        plugin plugin-install plugin-uninstall plugin-test smoke preset-test param-test scene-test preset-timing params scan icons clean clean-all reset-settings

help: ## Show this help
	@awk 'BEGIN {FS = ":.*## "; printf "\nUsage: make \033[36m<target>\033[0m [VAR=value]\n\n"} \
		/^[a-zA-Z_-]+:.*## / { printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@printf "\nVariables: FORMAT=$(FORMAT) PLUGIN=$(PLUGIN) (used by smoke, preset-test, param-test, scene-test, params); PRESET (plugin-test)\n\n"

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

build: node_modules ## Build frontend + release binary (UI embedded, no .app bundle)
	# Through the Tauri CLI: a plain `cargo build` leaves out the custom-protocol
	# feature, so the binary would load the dev server URL and show a blank window.
	npm run tauri build -- --no-bundle

# Tauri's (ad-hoc) signing step runs `xattr -cr`: make sure that's macOS's own xattr, not
# a Python one an asdf / pip install put earlier on PATH (it has no -r).
SYS_XATTR    := PATH="/usr/bin:$$PATH"

bundle: node_modules ## Build the macOS .app and .dmg
	$(SYS_XATTR) npm run tauri build

package: node_modules plugin ## Build the macOS installer (.pkg: app + VST3 + CLAP) into target/out
	$(SYS_XATTR) npm run tauri build -- --bundles app
	scripts/package-mac.sh "target/release/bundle/macos/Guitar Studio.app" target/plugin target/out

DOCS_DATA    := target/docs-data
docs-screenshots: node_modules ## Retake the user manual's screenshots (docs/manual/images; macOS + Chrome)
	npm run build
	$(CXX_FIX) cargo build --release -p guitar-studio --example docs_server
	node scripts/docs/prepare-data.mjs "$(APP_DATA)/plugins.json" $(DOCS_DATA)
	node scripts/docs/screenshots.mjs --data $(DOCS_DATA)

icons: node_modules ## Regenerate app icons from assets/app-icon.svg
	npx tauri icon assets/app-icon.svg -o src-tauri/icons
	rm -rf src-tauri/icons/android src-tauri/icons/ios src-tauri/icons/Square* src-tauri/icons/StoreLogo.png

# ---------------------------------------------------------------- plugin (DAW)

plugin: node_modules ## Build the VST3 + CLAP plugin bundles into target/plugin
	npm run build
	$(CXX_FIX) cargo build --release -p gs-plugin
	cargo build --release -p gs-engine --bin gs-scan
	scripts/bundle-plugin.sh target/release

plugin-install: plugin ## Build the plugin and install it for your DAWs (~/Library/Audio/Plug-Ins)
	@mkdir -p "$(PLUGIN_DIR)/VST3" "$(PLUGIN_DIR)/CLAP"
	rm -rf "$(PLUGIN_DIR)/VST3/Guitar Studio.vst3" "$(PLUGIN_DIR)/CLAP/Guitar Studio.clap"
	cp -R "target/plugin/Guitar Studio.vst3" "$(PLUGIN_DIR)/VST3/"
	cp -R "target/plugin/Guitar Studio.clap" "$(PLUGIN_DIR)/CLAP/"
	@echo "Installed. In REAPER: Options → Preferences → Plug-ins → VST → Re-scan (CLAP is scanned automatically)."

plugin-uninstall: ## Remove the installed plugin
	rm -rf "$(PLUGIN_DIR)/VST3/Guitar Studio.vst3" "$(PLUGIN_DIR)/CLAP/Guitar Studio.clap"

plugin-test: plugin ## Check the built plugin in a headless host (audio, Output, state; PRESET=name restores a preset)
	$(CXX_FIX) cargo run --release -q -p gs-plugin --example host_check -- \
		"target/plugin/Guitar Studio.clap" "target/plugin/Guitar Studio.vst3" \
		$(if $(PRESET),--preset "$(APP_DATA)/presets/$(PRESET).json")

# ---------------------------------------------------------------- quality

test: test-engine ## Run all tests
	npx tsc --noEmit

test-engine: ## Run Rust engine and app unit tests
	cargo test -p gs-engine -p gs-app

lint: node_modules ## Clippy + TypeScript type-check
	$(CXX_FIX) cargo clippy -p gs-engine -p gs-app -p gs-plugin -p guitar-studio --all-targets
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

scene-test: ## Switch scenes on a real plugin and save/reload them, e.g. make scene-test FORMAT=au PLUGIN=nolly
	cargo run -p gs-engine --example scene_switch -- $(FORMAT) "$(PLUGIN)"

preset-timing: ## Time plugin start-up vs. state restore, and preset switches, over your saved presets
	cargo run --release -p gs-engine --example preset_timing -- "$(APP_DATA)/presets"

params: ## List a plugin's parameters and ids, e.g. make params FORMAT=au PLUGIN=nolly
	cargo run -q -p gs-engine --example list_params -- $(FORMAT) "$(PLUGIN)"

scan: ## Rescan all plugins (crash-isolated) into the app's cache
	cargo build -p gs-engine --bin gs-scan
	@mkdir -p "$(APP_DATA)"
	cargo run -q -p gs-engine --example scan_catalog -- "$(CURDIR)/target/debug/gs-scan" "$(APP_DATA)/plugins.json"

# ---------------------------------------------------------------- housekeeping

reset-settings: ## Forget saved audio device settings and plugin cache
	rm -f "$(APP_DATA)/audio.json" "$(APP_DATA)/plugins.json"

clean: ## Remove build outputs
	cargo clean
	rm -rf dist

clean-all: clean ## Also remove node_modules
	rm -rf node_modules
