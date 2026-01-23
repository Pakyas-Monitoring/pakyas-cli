//! API types and display types for the check module.

use crate::cache::CheckLike;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use uuid::Uuid;

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Check {
    pub id: Uuid,
    pub public_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub period_seconds: i32,
    pub grace_seconds: i32,
    #[serde(default)]
    pub schedule_type: String,
    #[serde(default)]
    pub cron_expression: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    pub status: String,
    pub last_ping_at: Option<DateTime<Utc>>,
    pub next_ping_expected_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    // Lateness/alerting fields
    #[serde(default)]
    pub alert_after_failures: Option<i32>,
    #[serde(default)]
    pub consecutive_failures: i32,
    #[serde(default = "default_late_after_ratio")]
    pub late_after_ratio: f32,
    #[serde(default)]
    pub max_runtime_seconds: Option<i32>,
    #[serde(default = "default_missed_before_alert")]
    pub missed_before_alert: i32,
    // Additional fields from API
    #[serde(default)]
    pub soft_deleted: bool,
    #[serde(default)]
    pub alert_on_down: Option<bool>,
    #[serde(default)]
    pub alert_on_late: Option<bool>,
    #[serde(default)]
    pub alert_on_overrun: Option<bool>,
    #[serde(default)]
    pub alert_on_anomaly: Option<bool>,
    #[serde(default)]
    pub anomaly_status_performance: Option<String>,
    #[serde(default)]
    pub anomaly_status_reliability: Option<String>,
    #[serde(default)]
    pub status_before_start: Option<String>,
    #[serde(default)]
    pub notify_on_recovery: Option<bool>,
}

fn default_late_after_ratio() -> f32 {
    0.2
}

fn default_missed_before_alert() -> i32 {
    1
}

impl CheckLike for Check {
    fn id(&self) -> Uuid {
        self.id
    }
    fn public_id(&self) -> Uuid {
        self.public_id
    }
    fn slug(&self) -> &str {
        &self.slug
    }
    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Serialize)]
pub struct CreateCheckRequest {
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    pub period_seconds: i32,
    pub grace_seconds: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct UpdateCheckRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_after_failures: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub late_after_ratio: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_runtime_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_before_alert: Option<i32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PingLog {
    pub id: i64,
    #[serde(rename = "type")]
    pub ping_type: String,
    pub created_at: DateTime<Utc>,
    pub duration_ms: Option<i32>,
    pub source_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PingHistoryResponse {
    pub pings: Vec<PingLog>,
    pub total: i64,
}

// ============================================================================
// Display Types
// ============================================================================

/// Check with project name for org-wide listing
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheckWithProject {
    #[serde(flatten)]
    pub check: Check,
    pub project_name: String,
}

#[derive(Debug, Tabled, Serialize)]
pub struct CheckRowWithProject {
    #[tabled(rename = "PROJECT")]
    pub project: String,
    #[tabled(rename = "NAME")]
    pub name: String,
    #[tabled(rename = "SLUG")]
    pub slug: String,
    #[tabled(rename = "PUBLIC_ID")]
    pub public_id: String,
    #[tabled(rename = "STATUS")]
    pub status: String,
    #[tabled(rename = "PERIOD")]
    pub period: String,
    #[tabled(rename = "LAST PING")]
    pub last_ping: String,
}

#[derive(Debug, Tabled, Serialize)]
pub struct PingRow {
    #[tabled(rename = "TIME")]
    pub time: String,
    #[tabled(rename = "TYPE")]
    pub ping_type: String,
    #[tabled(rename = "DURATION")]
    pub duration: String,
    #[tabled(rename = "SOURCE")]
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct CheckDetail {
    pub id: String,
    pub public_id: String,
    pub name: String,
    pub slug: String,
    pub status: String,
    pub period: String,
    pub grace: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub last_ping: String,
    pub next_expected: String,
    pub ping_url: String,
    pub created_at: String,
}
