SHELL := /bin/bash

VERSION ?=
CHANNEL ?= rc
PUBLISH ?= 0

export VERSION
export CHANNEL
export PUBLISH

.DEFAULT_GOAL := help
.NOTPARALLEL:

.PHONY: help check release-version prerelease-version release-check \
	prerelease-check release prerelease

help:
	@echo "bootmux development and release targets"
	@echo
	@echo "  make check                         Run local quality gates"
	@echo "  make release-version               Print the next stable version"
	@echo "  make prerelease-version            Print the next prerelease version"
	@echo "  make release-check [VERSION=x.y.z] Validate a stable release without publishing"
	@echo "  make prerelease-check [VERSION=x.y.z-rc.1] [CHANNEL=rc]"
	@echo "                                     Validate a prerelease without publishing"
	@echo "  make release PUBLISH=1 [VERSION=x.y.z]"
	@echo "                                     Publish a stable release (default: patch + 1)"
	@echo "  make prerelease PUBLISH=1 [VERSION=x.y.z-rc.1] [CHANNEL=rc]"
	@echo "                                     Publish a prerelease (default: next rc)"

check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --locked -- -D warnings
	cargo test --all-targets --locked
	bash -n completion/bootmux.bash
	zsh -n completion/bootmux.zsh
	@command -v fish >/dev/null 2>&1 || { echo "error: fish is required to validate completion/bootmux.fish" >&2; exit 1; }
	fish -n completion/bootmux.fish
	git diff --check

release-version:
	@bash scripts/release.sh version stable "$$VERSION" "$$CHANNEL"

prerelease-version:
	@bash scripts/release.sh version prerelease "$$VERSION" "$$CHANNEL"

release-check:
	@bash scripts/release.sh check stable "$$VERSION" "$$CHANNEL"

prerelease-check:
	@bash scripts/release.sh check prerelease "$$VERSION" "$$CHANNEL"

release:
	@BOOTMUX_PUBLISH="$$PUBLISH" bash scripts/release.sh publish stable "$$VERSION" "$$CHANNEL"

prerelease:
	@BOOTMUX_PUBLISH="$$PUBLISH" bash scripts/release.sh publish prerelease "$$VERSION" "$$CHANNEL"
