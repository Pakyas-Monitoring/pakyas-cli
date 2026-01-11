# Pakyas CLI

[![CI](https://github.com/pakyas/pakyas-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/pakyas/pakyas-cli/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

The official command-line interface for [Pakyas](https://pakyas.com) - a cron job and scheduled task monitoring service.

## Installation

### From Source (Cargo)

```bash
# Clone the repository
git clone https://github.com/pakyas/pakyas-cli.git
cd pakyas-cli

# Build and install
cargo install --path .
```

### Homebrew (macOS/Linux)

```bash
# Coming soon
brew install pakyas/tap/pakyas
```

### Binary Download

Pre-built binaries for Linux and macOS are available on the [Releases](https://github.com/pakyas/pakyas-cli/releases) page.

```bash
# Example for Linux x86_64
curl -LO https://github.com/pakyas/pakyas-cli/releases/latest/download/pakyas-0.1.0-x86_64-unknown-linux-gnu.tar.gz
tar xzf pakyas-0.1.0-x86_64-unknown-linux-gnu.tar.gz
sudo mv pakyas /usr/local/bin/
```

## Quick Start

```bash
# 1. Login with your API key
pakyas login --api-key pk_live_your_api_key_here

# 2. Select your organization and project
pakyas org switch "My Company"
pakyas project switch "Production"

# 3. List your checks
pakyas check list

# 4. Send a ping
pakyas ping my-backup-job

# 5. Wrap a command with monitoring
pakyas monitor daily-backup -- /opt/scripts/backup.sh
```

## Command Reference

### Authentication

| Command | Description |
|---------|-------------|
| `pakyas login` | Interactive login (browser-based) |
| `pakyas login --api-key <KEY>` | Login with API key directly |
| `pakyas logout` | Clear stored credentials |
| `pakyas whoami` | Show current user and context |

### Organizations

| Command | Description |
|---------|-------------|
| `pakyas org list` | List all organizations you belong to |
| `pakyas org switch <NAME>` | Set active organization |

### Projects

| Command | Description |
|---------|-------------|
| `pakyas project list` | List projects in active organization |
| `pakyas project create` | Create a new project (interactive) |
| `pakyas project create --name "Prod"` | Create project with flags |
| `pakyas project switch <NAME>` | Set active project |

### Checks

| Command | Description |
|---------|-------------|
| `pakyas check list` | List all checks in active project |
| `pakyas check create` | Create a check (interactive) |
| `pakyas check create --name "Backup" --slug backup --period 3600` | Create with flags |
| `pakyas check show <SLUG>` | Show check details and ping URL |
| `pakyas check pause <SLUG>` | Pause monitoring |
| `pakyas check resume <SLUG>` | Resume monitoring |
| `pakyas check delete <SLUG>` | Delete check (with confirmation) |
| `pakyas check logs <SLUG>` | Show ping history |
| `pakyas check logs <SLUG> --limit 100` | Show more history |
| `pakyas check sync` | Force refresh local cache |

### Pings

| Command | Description |
|---------|-------------|
| `pakyas ping <SLUG>` | Send success ping |
| `pakyas ping <SLUG> --start` | Signal job started |
| `pakyas ping <SLUG> --fail` | Signal job failed |
| `pakyas ping <SLUG> --exit-code <N>` | Send with specific exit code |

### Monitor (Command Wrapper)

Wrap any command with automatic start/success/fail pings:

```bash
pakyas monitor <SLUG> -- <COMMAND> [ARGS...]
```

The monitor command:
1. Sends a `/start` ping before the command runs
2. Executes your command
3. Sends a success ping if exit code is 0, or fail ping otherwise
4. Returns the same exit code as your command

### API Keys

| Command | Description |
|---------|-------------|
| `pakyas api-key list` | List API keys in active organization |
| `pakyas api-key create <NAME>` | Create a new API key |
| `pakyas api-key create <NAME> --scopes read,write` | Create with specific scopes |
| `pakyas api-key revoke <ID>` | Revoke an API key |

### Shell Completions

| Command | Description |
|---------|-------------|
| `pakyas completion bash` | Generate bash completions |
| `pakyas completion zsh` | Generate zsh completions |
| `pakyas completion fish` | Generate fish completions |

### Update

| Command | Description |
|---------|-------------|
| `pakyas update` | Update to the latest CLI version |
| `pakyas update --check` | Check if an update is available |

The CLI automatically checks for updates in the background (except during `ping`, `monitor`, and `completion` commands) and shows a notice if a newer version is available.

## Global Options

These options work with any command:

| Option | Description |
|--------|-------------|
| `--org <ORG>` | Override active organization for this command |
| `--project <PROJECT>` | Override active project for this command |
| `--format <FORMAT>` | Output format: `table` (default) or `json` |
| `-v, --verbose` | Enable verbose output |
| `-h, --help` | Show help |
| `-V, --version` | Show version |

## Configuration

### Config File

Configuration is stored at `~/.config/pakyas/config.toml`:

```toml
api_url = "https://api.pakyas.com"
ping_url = "https://ping.pakyas.com"
active_org_id = "uuid-here"
active_project_id = "uuid-here"
format = "table"
color = true
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `PAKYAS_API_KEY` | API key (overrides stored credentials) |
| `PAKYAS_ORG` | Override active organization |
| `PAKYAS_PROJECT` | Override active project |
| `API_URL` | Override API URL |
| `PING_URL` | Override ping URL |
| `PAKYAS_FORMAT` | Output format (`table` or `json`) |
| `NO_COLOR` | Disable colored output |

### Configuration Precedence

1. **CLI flags** (`--org`, `--project`, `--format`)
2. **Environment variables** (`PAKYAS_ORG`, etc.)
3. **Config file** (`~/.config/pakyas/config.toml`)
4. **Built-in defaults**

## CI/CD Usage

### Environment Setup

```bash
# Set your API key as a secret in your CI system
export PAKYAS_API_KEY=pk_live_your_api_key_here
export PAKYAS_PROJECT=production
```

### GitHub Actions Example

```yaml
name: Backup Job
on:
  schedule:
    - cron: '0 2 * * *'  # Daily at 2 AM

jobs:
  backup:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Pakyas CLI
        run: |
          curl -LO https://github.com/pakyas/pakyas-cli/releases/latest/download/pakyas-0.1.0-x86_64-unknown-linux-gnu.tar.gz
          tar xzf pakyas-0.1.0-x86_64-unknown-linux-gnu.tar.gz
          sudo mv pakyas /usr/local/bin/

      - name: Run Backup with Monitoring
        env:
          PAKYAS_API_KEY: ${{ secrets.PAKYAS_API_KEY }}
          PAKYAS_PROJECT: ${{ vars.PAKYAS_PROJECT }}
        run: pakyas monitor daily-backup -- ./scripts/backup.sh
```

### Crontab Examples

```cron
# Monitor a backup script
0 2 * * * PAKYAS_API_KEY=pk_live_xxx pakyas monitor daily-backup -- /opt/scripts/backup.sh

# Monitor a sync job
0 * * * * PAKYAS_API_KEY=pk_live_xxx pakyas monitor hourly-sync -- python /app/sync.py

# Simple ping after a job completes
30 3 * * * /opt/scripts/cleanup.sh && PAKYAS_API_KEY=pk_live_xxx pakyas ping cleanup-job
```

## Shell Completions

### Bash

```bash
# Add to ~/.bashrc
pakyas completion bash > ~/.bash_completion.d/pakyas
source ~/.bash_completion.d/pakyas
```

### Zsh

```bash
# Add to ~/.zshrc (before compinit)
pakyas completion zsh > ~/.zfunc/_pakyas
fpath+=~/.zfunc
autoload -Uz compinit && compinit
```

### Fish

```bash
pakyas completion fish > ~/.config/fish/completions/pakyas.fish
```

## Troubleshooting

### "Not logged in"

Run `pakyas login` or set the `PAKYAS_API_KEY` environment variable.

### "No project selected"

Run `pakyas project switch <NAME>` or set `PAKYAS_PROJECT` environment variable.

### "Check not found"

The check slug may not exist in your current project. Run `pakyas check list` to see available checks, or use `pakyas check sync` to refresh the local cache.

### API connection issues

Check your network connection and verify the API URL:
```bash
pakyas whoami --verbose
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### Development Setup

```bash
# Clone the repository
git clone https://github.com/pakyas/pakyas-cli.git
cd pakyas-cli

# Build (uses production API URLs by default)
cargo build

# Or build with custom URLs for local development
API_URL=http://localhost:8080 PING_URL=http://localhost:8787 cargo build

# Run tests
cargo test

# Run clippy
cargo clippy

# Run coverage (Linux only, or use Docker on macOS)
cargo install cargo-tarpaulin
cargo tarpaulin --all-features
```

### Architecture

See [ARCHITECTURE.md](./ARCHITECTURE.md) for details on the codebase structure.

## Links

- [Pakyas Website](https://pakyas.com)
- [API Documentation](https://docs.pakyas.com)
- [GitHub Issues](https://github.com/pakyas/pakyas-cli/issues)

## License

MIT - see [LICENSE](./LICENSE) for details.
