# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build                    # Build (uses prod API URLs by default)
cargo build --release          # Release build
cargo test                     # Run all tests
cargo test <test_name>         # Run a single test
cargo clippy                   # Run linter
cargo tarpaulin --all-features # Run coverage (Linux only)
cargo install --path .         # Install locally
```

For local development with custom API endpoints:
```bash
API_URL=http://localhost:8080 PING_URL=http://localhost:8787 cargo build
```

## Architecture

Pakyas CLI is a Rust CLI for a cron job monitoring service. It uses the heartbeat pattern - jobs send pings to Pakyas, and if expected pings don't arrive, alerts are triggered.

### Data Hierarchy

Organization > Project > Check (strict containment, CLI reflects this in command structure)

### Module Structure

- `src/cli.rs` - Command definitions using clap with derive macros. Global flags (`--org`, `--project`, `--format`) are flattened into all subcommands.
- `src/config.rs` - Context management. Loads from `~/.config/pakyas/config.toml`, handles override precedence (CLI flags > env vars > config file > defaults).
- `src/commands/` - Command handlers organized by resource (auth, org, project, check, ping, monitor, api_key, update, completion).
- `src/client.rs` - HTTP client for API calls.
- `src/cache.rs` - Local caching of check slugs→UUIDs for fast ping lookups.
- `src/credentials.rs` - Token storage.
- `src/external_monitors.rs`, `src/external_ping.rs` - Integration with external monitoring services (healthchecks.io, cronitor, webhooks).
- `src/update_cache.rs` - Background update checking (skipped for hot-path commands: ping, monitor, completion).

### Key Patterns

**Context Resolution**: The `config::Context` struct handles the precedence pyramid - CLI flags override env vars, which override config file values. Commands receive a resolved context.

**Hot-path optimization**: `ping` and `monitor` commands skip update checks and use cached check UUIDs to minimize latency.

**Monitor command**: Wraps external commands with automatic start/success/fail pings. Returns the wrapped command's exit code.

### Config File Location

`~/.config/pakyas/config.toml` stores API URLs, active org/project IDs, and format preferences.

### Environment Variables

- `PAKYAS_API_KEY` - API key (overrides stored credentials)
- `PAKYAS_ORG` / `PAKYAS_PROJECT` - Override active org/project
- `API_URL` / `PING_URL` - Override API endpoints
- `PAKYAS_FORMAT` - Output format (table/json)

## Commit Convention

This project uses [Conventional Commits](https://www.conventionalcommits.org/) for changelog generation.

```
<type>[optional scope]: <description>
```

**Types:**
- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation only
- `perf` - Performance improvement
- `refactor` - Code change that neither fixes a bug nor adds a feature
- `test` - Adding or updating tests
- `chore` - Maintenance tasks, CI changes

**Examples:**
```bash
feat(ping): add --timeout flag
fix(auth): handle expired tokens gracefully
docs: update installation instructions
perf(cache): optimize slug lookup
```

**Breaking changes:** Add `!` after the type:
```bash
feat!: change default output format to JSON
```

## Release Process

1. Update version in `Cargo.toml`
2. Commit: `chore(release): prepare v0.x.x`
3. Tag and push: `git tag v0.x.x && git push --tags`
4. GitHub Actions builds and creates the release with changelog
