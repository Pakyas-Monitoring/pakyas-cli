use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "pakyas")]
#[command(author, version, about = "CLI for Pakyas cron monitoring service")]
#[command(propagate_version = true)]
pub struct Cli {
    /// Override active organization
    #[arg(long, global = true, env = "PAKYAS_ORG")]
    pub org: Option<String>,

    /// Override active project
    #[arg(long, global = true, env = "PAKYAS_PROJECT")]
    pub project: Option<String>,

    /// Output format
    #[arg(long, global = true, value_enum, env = "PAKYAS_FORMAT")]
    pub format: Option<OutputFormat>,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Disable update check
    #[arg(long, global = true, env = "PAKYAS_NO_UPDATE_CHECK")]
    pub no_update_check: bool,

    /// Ignore PAKYAS_API_KEY environment variable for this command
    #[arg(long, global = true)]
    pub ignore_env: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Login to Pakyas
    Login(LoginArgs),

    /// Logout and clear credentials
    Logout,

    /// Show current user and context
    Whoami,

    /// Organization management
    #[command(subcommand)]
    Org(OrgCommands),

    /// Project management
    #[command(subcommand)]
    Project(ProjectCommands),

    /// Check management
    #[command(subcommand)]
    Check(CheckCommands),

    /// Send a ping to a check
    Ping(PingArgs),

    /// Wrap a command with monitoring (sends start/success/fail pings)
    Monitor(MonitorArgs),

    /// API key management
    #[command(subcommand)]
    ApiKey(ApiKeyCommands),

    /// Authentication management (credentials, status)
    #[command(subcommand)]
    Auth(AuthCommands),

    /// Generate shell completions
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Update pakyas to the latest version
    Update(UpdateArgs),
}

#[derive(Args, Clone)]
pub struct UpdateArgs {
    /// Check for updates without installing
    #[arg(long)]
    pub check: bool,
}

#[derive(Parser, Clone)]
pub struct LoginArgs {
    /// Login with API key directly (skip browser auth)
    #[arg(long)]
    pub api_key: Option<String>,

    /// Skip browser and use email/password interactively
    #[arg(long)]
    pub no_browser: bool,
}

#[derive(Subcommand, Clone)]
pub enum OrgCommands {
    /// List organizations you belong to
    List,

    /// Switch active organization
    Switch {
        /// Organization name or ID
        name: String,

        /// Fail if no key stored (for CI/scripts, disables interactive prompts)
        #[arg(long)]
        no_prompt: bool,
    },
}

#[derive(Subcommand, Clone)]
pub enum ProjectCommands {
    /// List projects in the active organization
    List,

    /// Create a new project
    Create {
        /// Project name
        #[arg(long)]
        name: Option<String>,

        /// Project description
        #[arg(long)]
        description: Option<String>,
    },

    /// Switch active project
    Switch {
        /// Project name or ID (use quotes for names with spaces)
        #[arg(num_args = 1..)]
        name: Vec<String>,
    },
}

#[derive(Subcommand, Clone)]
pub enum CheckCommands {
    /// List all checks in the active project
    List,

    /// Create a new check
    Create {
        /// Check slug (URL-friendly identifier)
        slug: String,

        /// Display name (defaults to title-cased slug)
        #[arg(long)]
        name: Option<String>,

        /// Cron expression (e.g., "0 2 * * *" for daily at 2am)
        #[arg(
            long,
            alias = "schedule",
            value_name = "CRON",
            conflicts_with = "every"
        )]
        cron: Option<String>,

        /// Timezone for cron (IANA format, e.g., "Asia/Manila")
        #[arg(long, value_name = "TZ", requires = "cron")]
        tz: Option<String>,

        /// Expected ping interval (e.g., "5m", "1h", "30s")
        #[arg(long, value_name = "DURATION", conflicts_with = "cron")]
        every: Option<String>,

        /// Grace period before marking as missed (e.g., "10m", "30s"). Auto-derived if not specified.
        #[arg(long, value_name = "DURATION")]
        grace: Option<String>,

        /// Check description
        #[arg(long)]
        description: Option<String>,

        /// Output as JSON (compact, for scripting)
        #[arg(long)]
        json: bool,

        /// Only print the ping URL
        #[arg(long)]
        quiet: bool,

        /// Show what would be created without creating
        #[arg(long)]
        dry_run: bool,

        /// Force interactive mode
        #[arg(short, long)]
        interactive: bool,
    },

    /// Show check details
    Show {
        /// Check slug or ID
        slug: String,
    },

    /// Pause a check (stops monitoring)
    Pause {
        /// Check slug or ID
        slug: String,
    },

    /// Resume a paused check
    Resume {
        /// Check slug or ID
        slug: String,
    },

    /// Delete a check
    Delete {
        /// Check slug or ID
        slug: String,

        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },

    /// Show ping history for a check
    Logs {
        /// Check slug or ID
        slug: String,

        /// Number of pings to show (default: 50)
        #[arg(long, default_value = "50")]
        limit: i32,
    },

    /// Force refresh the local check cache
    Sync,

    /// Update a check's configuration
    Update {
        /// Check slug or ID
        slug: String,

        /// New check name
        #[arg(long)]
        name: Option<String>,

        /// New description (use "" to clear)
        #[arg(long)]
        description: Option<String>,

        /// Cron expression (e.g., "0 2 * * *"). Use "" to clear and switch to interval mode.
        #[arg(long, alias = "schedule", conflicts_with = "every")]
        cron: Option<String>,

        /// Timezone for cron (IANA format, e.g., "Asia/Manila"). Use "" to clear.
        #[arg(long, value_name = "TZ")]
        tz: Option<String>,

        /// Period/interval (e.g., 5m, 1h, 1d, or raw seconds)
        #[arg(long, alias = "period", conflicts_with = "cron")]
        every: Option<String>,

        /// Grace period (e.g., 30s, 5m, or raw seconds)
        #[arg(long)]
        grace: Option<String>,

        /// Tags (comma-separated, replaces existing)
        #[arg(long)]
        tags: Option<String>,

        /// Alert after N consecutive failures (1-100)
        #[arg(long)]
        alert_after_failures: Option<i32>,

        /// Late threshold as ratio of period (0.0-1.0)
        #[arg(long)]
        late_after_ratio: Option<f32>,

        /// Maximum runtime (e.g., 5m, 10m, or raw seconds)
        #[arg(long)]
        max_runtime: Option<String>,

        /// Alert after N consecutive misses (1-100)
        #[arg(long)]
        missed_before_alert: Option<i32>,

        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Args, Clone)]
pub struct PingArgs {
    /// Check slug to ping (required unless --public_id is provided)
    #[arg(required_unless_present = "public_id")]
    pub slug: Option<String>,

    /// Check public ID (UUID) - skips authentication and slug resolution
    #[arg(long, env = "PAKYAS_PUBLIC_ID")]
    pub public_id: Option<Uuid>,

    /// Send a "start" signal (job started)
    #[arg(long, conflicts_with_all = ["fail", "exit_code"])]
    pub start: bool,

    /// Send a "fail" signal (job failed)
    #[arg(long, conflicts_with_all = ["start", "exit_code"])]
    pub fail: bool,

    /// Send with exit code (0 = success, non-zero = fail)
    #[arg(long, conflicts_with_all = ["start", "fail"])]
    pub exit_code: Option<i32>,

    /// Run identifier for START/END pairing (enables accurate duration tracking)
    /// Use the same ID for --start and completion pings to pair them together
    #[arg(long)]
    pub run: Option<String>,

    /// Duration in milliseconds (for scripted pings with accurate timing)
    /// Only used with completion pings (not --start)
    #[arg(long, conflicts_with = "start")]
    pub duration_ms: Option<u64>,

    /// Disable external monitors (healthchecks.io, cronitor, webhooks)
    #[arg(long, env = "PAKYAS_NO_EXTERNAL")]
    pub no_external: bool,

    /// Timeout for external monitor requests in milliseconds
    #[arg(long, default_value = "5000", env = "PAKYAS_EXTERNAL_TIMEOUT_MS")]
    pub external_timeout_ms: u64,
}

#[derive(Args, Clone)]
pub struct MonitorArgs {
    /// Check slug to monitor (required unless --public_id is provided)
    #[arg(required_unless_present = "public_id")]
    pub slug: Option<String>,

    /// Check public ID (UUID) - skips authentication and slug resolution
    #[arg(long, env = "PAKYAS_PUBLIC_ID")]
    pub public_id: Option<Uuid>,

    /// Command to execute (everything after --)
    #[arg(last = true, required = true)]
    pub command: Vec<String>,

    /// Disable external monitors (healthchecks.io, cronitor, webhooks)
    #[arg(long, env = "PAKYAS_NO_EXTERNAL")]
    pub no_external: bool,

    /// Timeout for external monitor requests in milliseconds
    #[arg(long, default_value = "5000", env = "PAKYAS_EXTERNAL_TIMEOUT_MS")]
    pub external_timeout_ms: u64,

    /// Migration mode: allow external success to override pakyas failure
    #[arg(long, env = "PAKYAS_MIGRATION_MODE")]
    pub migration_mode: bool,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum OutputFormat {
    /// Display as formatted table
    #[default]
    Table,
    /// Display as JSON
    Json,
}

#[derive(Subcommand, Clone)]
pub enum ApiKeyCommands {
    /// List all API keys in the active organization
    List,

    /// Create a new API key
    Create {
        /// Name for the API key (e.g., "CI Pipeline", "Local Dev")
        name: String,

        /// Scopes to grant: read, write, manage (can specify multiple with commas)
        #[arg(long, short, value_delimiter = ',')]
        scopes: Vec<String>,

        /// Days until expiration (1-365, omit for no expiration)
        #[arg(long)]
        expires: Option<i64>,
    },

    /// Revoke an API key
    Revoke {
        /// API key ID to revoke
        id: String,

        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand, Clone)]
pub enum AuthCommands {
    /// Show authentication status and credential info
    Status,

    /// Manage stored API keys for organizations
    #[command(subcommand)]
    Key(AuthKeyCommands),
}

#[derive(Subcommand, Clone)]
pub enum AuthKeyCommands {
    /// List all stored API keys by organization
    List,

    /// Set/import an API key for an organization
    Set {
        /// Organization ID to set key for
        #[arg(long)]
        org: String,

        /// API key to store (will prompt if not provided)
        #[arg(long)]
        key: Option<String>,
    },

    /// Verify a stored API key is valid
    Verify {
        /// Organization ID to verify key for (uses active org if not specified)
        #[arg(long)]
        org: Option<String>,
    },

    /// Remove a stored API key
    Rm {
        /// Organization ID to remove key for
        #[arg(long, conflicts_with = "legacy")]
        org: Option<String>,

        /// Remove the legacy (unmigrated) API key
        #[arg(long, conflicts_with = "org")]
        legacy: bool,

        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
}
