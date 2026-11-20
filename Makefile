SHELL := /bin/sh
UNAME_S := $(shell uname -s)

.PHONY: build fmt fmt-check lint test test-unit test-integration test-e2e check bench bench-test ci ci-macos hooks-install install-local run-local

build:
	cargo build --release
	mkdir -p bin
ifneq ($(UNAME_S),Darwin)
	cp target/release/doctty bin/doctty
else
	codesign --force --sign - target/release/doctty
	cp target/release/doctty bin/doctty
	cp target/release/doctty-renderer bin/doctty-renderer
	codesign --force --sign - bin/doctty-renderer
	codesign --force --sign - bin/doctty
endif


fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --workspace --all-targets --all-features

test-unit:
	cargo test --test unit --all-features

test-integration:
	cargo test --test integration --all-features

test-e2e:
	cargo test --test e2e --all-features

check:
	cargo check --workspace --all-targets --all-features

bench:
	python3 bench/run.py

bench-test:
	python3 -m unittest discover -s bench -p 'test_*.py'

ci: fmt-check lint test bench-test

# The renderer is macOS-only. Build with --locked before `lint` so a stale
# Cargo.lock fails here instead of being rewritten by Clippy.
ci-macos:
	cargo build --release --locked
	$(MAKE) lint
	cargo test --locked --all-features --bin doctty-renderer

hooks-install:
	prek install --overwrite

install-local:
	cargo install --path . --force

run-local: build
	./bin/doctty $(FILE)
