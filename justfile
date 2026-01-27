# Default recipe - show available commands
default:
    @just --list

# Load environment variables from .env file
set dotenv-load

# Build debug binary with local URLs
build:
    cargo build

# Build release binary with local URLs
build-release:
    cargo build --release

# Run the CLI (pass arguments after --)
run *ARGS:
    cargo run -- {{ARGS}}

# Install locally with local URLs
install:
    cargo install --path .

# Run all tests
test:
    cargo test --all-features

# Run a specific test
test-one NAME:
    cargo test {{NAME}} -- --nocapture

# Run unit tests only
test-unit:
    cargo test --lib

# Run integration tests only
test-integration:
    cargo test --test '*'

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt --check

# Run clippy with strict warnings
clippy:
    cargo clippy -- -D warnings

# Run clippy and auto-fix
clippy-fix:
    cargo clippy --fix --allow-dirty

# Full check (format + lint + test)
check: fmt-check clippy test

# Quick compile check (no codegen)
compile-check:
    cargo check

# Clean build artifacts
clean:
    cargo clean

# Prepare and push a release (usage: just release 0.1.8)
release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail

    # Update version in Cargo.toml
    sed -i '' 's/^version = ".*"/version = "{{VERSION}}"/' Cargo.toml

    # Update Cargo.lock
    cargo check --quiet

    # Commit the version bump
    git add Cargo.toml Cargo.lock
    git commit -m "chore(release): prepare v{{VERSION}}"

    # Create tag
    git tag "v{{VERSION}}"

    # Push branch and tag
    git push
    git push origin "v{{VERSION}}"

    echo "Released v{{VERSION}}"
