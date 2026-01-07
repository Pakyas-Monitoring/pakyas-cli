//! Cron expression utilities for schedule calculation

use cron::Schedule;
use std::str::FromStr;

/// Normalize a cron expression to 6-field format.
/// The `cron` crate requires 6 fields (sec min hour day month weekday),
/// but users often provide 5 fields (min hour day month weekday).
fn normalize_cron_expression(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() == 5 {
        // Prepend "0" for seconds field
        format!("0 {}", expr)
    } else {
        expr.to_string()
    }
}

/// Calculate the effective period in seconds for a cron expression.
/// Returns the MINIMUM gap between any two consecutive executions
/// across a week's worth of runs. This makes the result deterministic
/// regardless of when it's called.
///
/// Examples:
/// - `0 2 * * *` (daily at 2am): min gap = 86400 (24h)
/// - `0 2,14 * * *` (2am and 2pm): min gap = 43200 (12h)
/// - `0 1,2 * * *` (1am and 2am): min gap = 3600 (1h, not 23h)
pub fn effective_period_from_cron(cron_expression: &str) -> Option<i32> {
    let normalized = normalize_cron_expression(cron_expression);
    let schedule = Schedule::from_str(&normalized).ok()?;

    // Use current time as starting point (doesn't affect result since we find min)
    let start = chrono::Utc::now();

    // Get a week's worth of occurrences (enough to capture all patterns)
    let occurrences: Vec<_> = schedule.after(&start).take(168).collect(); // 168 = 24*7

    if occurrences.len() < 2 {
        return None;
    }

    // Find minimum gap between consecutive occurrences
    let min_gap = occurrences
        .windows(2)
        .map(|w| (w[1] - w[0]).num_seconds())
        .min()?;

    Some(min_gap as i32)
}

/// Validate a cron expression.
pub fn validate_cron_expression(expr: &str) -> Result<(), String> {
    let normalized = normalize_cron_expression(expr);
    Schedule::from_str(&normalized)
        .map(|_| ())
        .map_err(|e| format!("Invalid cron expression: {}", e))
}

/// Get next N cron times in the specified timezone.
/// Used for dry-run display to show upcoming executions in the user's timezone.
pub fn next_cron_times_in_tz(
    cron_expr: &str,
    tz: chrono_tz::Tz,
    count: usize,
) -> Vec<chrono::DateTime<chrono_tz::Tz>> {
    let normalized = normalize_cron_expression(cron_expr);
    let schedule = match Schedule::from_str(&normalized) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let now_utc = chrono::Utc::now();
    schedule
        .after(&now_utc)
        .take(count)
        .map(|dt| dt.with_timezone(&tz))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_cron() {
        let result = validate_cron_expression("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_cron() {
        // Standard 6-field cron: sec min hour day month weekday
        let result = validate_cron_expression("0 0 * * * *");
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_cron_5_field() {
        // Standard 5-field cron: min hour day month weekday (should be normalized)
        let result = validate_cron_expression("0 * * * *");
        assert!(result.is_ok());
    }

    #[test]
    fn test_effective_period_hourly() {
        // Every hour (6-field)
        let period = effective_period_from_cron("0 0 * * * *");
        assert!(period.is_some());
        assert_eq!(period.unwrap(), 3600);
    }

    #[test]
    fn test_effective_period_hourly_5_field() {
        // Every hour (5-field, normalized to 6-field)
        let period = effective_period_from_cron("0 * * * *");
        assert!(period.is_some());
        assert_eq!(period.unwrap(), 3600);
    }

    #[test]
    fn test_effective_period_every_5_min() {
        // Every 5 minutes
        let period = effective_period_from_cron("0 */5 * * * *");
        assert!(period.is_some());
        assert_eq!(period.unwrap(), 300);
    }

    #[test]
    fn test_effective_period_irregular_cron() {
        // Runs at 1am and 2am - should return 1 hour (min gap), not 23 hours
        let period = effective_period_from_cron("0 1,2 * * *");
        assert!(period.is_some());
        assert_eq!(period.unwrap(), 3600); // 1 hour, not 23 hours
    }

    #[test]
    fn test_effective_period_twice_daily() {
        // Runs at 2am and 2pm - min gap is 12 hours
        let period = effective_period_from_cron("0 2,14 * * *");
        assert!(period.is_some());
        assert_eq!(period.unwrap(), 43200); // 12 hours
    }

    #[test]
    fn test_next_cron_times_in_tz() {
        use chrono_tz::Asia::Manila;

        let times = next_cron_times_in_tz("0 2 * * *", Manila, 3);
        assert_eq!(times.len(), 3);

        // All times should be in Manila timezone
        for time in &times {
            assert_eq!(time.timezone(), Manila);
        }
    }
}
