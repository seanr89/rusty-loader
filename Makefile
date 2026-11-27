APP_NAME ?= rusty-loader
APP_DIR ?= /Applications
BUNDLE_ID ?= com.seanr89.rusty-loader
CODESIGN_IDENTITY ?= -
VERSION := $(shell awk -F'"' '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^version[[:space:]]*=/{print $$2; exit}' Cargo.toml)
BUNDLE_SCRIPT := ./scripts/macos_bundle.sh

BUNDLE_ENV := \
	APP_NAME="$(APP_NAME)" \
	APP_DIR="$(APP_DIR)" \
	BUNDLE_ID="$(BUNDLE_ID)" \
	CODESIGN_IDENTITY="$(CODESIGN_IDENTITY)" \
	VERSION="$(VERSION)"

.PHONY: help build macos-icon macos-bundle macos-install macos-update macos-uninstall

help:
	@echo "rusty-loader (v$(VERSION)) Makefile targets:"
	@echo ""
	@echo "  make build            Build the release binary (cargo build --release)"
	@echo "  make macos-icon       Generate packaging/macos/rusty-loader.icns from master icon"
	@echo "  make macos-bundle     Build and package target/release/bundle/$(APP_NAME).app"
	@echo "  make macos-install    Install signed $(APP_NAME).app into $(APP_DIR)"
	@echo "  make macos-update     Rebuild and atomically update an existing $(APP_NAME).app in $(APP_DIR)"
	@echo "  make macos-uninstall  Remove $(APP_NAME).app from $(APP_DIR)"
	@echo ""
	@echo "Configurable variables:"
	@echo "  APP_NAME=$(APP_NAME)"
	@echo "  APP_DIR=$(APP_DIR)"
	@echo "  BUNDLE_ID=$(BUNDLE_ID)"
	@echo "  CODESIGN_IDENTITY=$(CODESIGN_IDENTITY)"

build:
	cargo build --release

macos-icon:
	./scripts/generate_macos_icon.sh

macos-bundle: build
	$(BUNDLE_ENV) $(BUNDLE_SCRIPT) bundle

macos-install: macos-bundle
	$(BUNDLE_ENV) $(BUNDLE_SCRIPT) install

macos-update:
	$(BUNDLE_ENV) $(BUNDLE_SCRIPT) check-installed
	$(MAKE) macos-bundle APP_NAME="$(APP_NAME)" BUNDLE_ID="$(BUNDLE_ID)" VERSION="$(VERSION)"
	$(BUNDLE_ENV) $(BUNDLE_SCRIPT) update

macos-uninstall:
	$(BUNDLE_ENV) $(BUNDLE_SCRIPT) uninstall
