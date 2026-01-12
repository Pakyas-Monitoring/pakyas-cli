use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("Not logged in. Run: pakyas login")]
    NotAuthenticated,

    #[error("No organization selected. Run: pakyas org switch <NAME>")]
    NoOrgSelected,

    #[error("No project selected. Run: pakyas project switch <NAME>")]
    NoProjectSelected,

    #[error("Organization '{0}' not found. Run: pakyas org list")]
    OrgNotFound(String),

    #[error("Project '{0}' not found. Run: pakyas project list")]
    ProjectNotFound(String),

    #[error("Check '{0}' not found. Run: pakyas check list")]
    CheckNotFound(String),

    #[error("Invalid API key format. Keys should start with 'pk_'")]
    InvalidApiKey,

    #[error("API error: {0}")]
    Api(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Failed to read config: {0}")]
    ConfigRead(std::io::Error),

    #[error("Failed to write config: {0}")]
    ConfigWrite(std::io::Error),

    #[error("Invalid config format: {0}")]
    ConfigParse(#[from] toml::de::Error),

    #[error("Failed to serialize config: {0}")]
    ConfigSerialize(#[from] toml::ser::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Credentials file is corrupted. Run 'pakyas login' or 'pakyas auth key set --org <org>' to fix.")]
    CredentialsCorrupted,

    #[error("Failed to acquire config lock. Another pakyas process may be running.")]
    LockFailed,

    #[error("State changed in another terminal: {0}")]
    ConcurrentModification(String),

    #[error("No API key for organization '{0}'. Run: pakyas org switch {0}")]
    NoKeyForOrg(String),

    #[error("API key belongs to org '{key_org}' but active org is '{active_org}'.\nRun: pakyas auth key create --org {active_org}\nOr:  pakyas org switch {key_org}")]
    OrgKeyMismatch { key_org: String, active_org: String },

    #[error("Cannot switch org while PAKYAS_API_KEY is set.\nEither: unset PAKYAS_API_KEY, or pass --ignore-env")]
    EnvKeyBlocksSwitch,

    #[error("{0}")]
    Other(String),
}

impl CliError {
    pub fn api(msg: impl Into<String>) -> Self {
        Self::Api(msg.into())
    }
}
