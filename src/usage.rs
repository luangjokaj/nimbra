//! Anonymous usage counts: how many Macs open Nimbra each day, and how often it is opened.
//!
//! Each launch sends one event to PostHog (EU) with the app's version, the macOS version and
//! whether this is the first launch on this Mac today. Nothing else: no ID that lasts beyond
//! the one event, no address, nothing about accounts or mail. The event's `distinct_id` is
//! random every time and PostHog keeps no person profile for it, so two launches cannot be
//! linked. Daily users are the launches marked `first_today`; opens are all of them.
//!
//! Only builds made with `NIMBRA_POSTHOG_KEY` set send anything (the release builds; see
//! `scripts/bundle-macos.sh`), and never debug builds, test runs or when the switch in
//! Settings › Privacy is off.

use std::path::PathBuf;
use std::time::Duration;

/// PostHog's capture endpoint for the EU region.
const ENDPOINT: &str = "https://eu.i.posthog.com/i/v0/e/";

/// The project's write-only key, given at build time.
const KEY: Option<&str> = option_env!("NIMBRA_POSTHOG_KEY");

/// Counts this launch, off the main thread. Does nothing when counting is off or not built in.
pub fn opened(enabled: bool) {
    let Some(key) = KEY.map(str::trim).filter(|k| k.starts_with("phc_")) else {
        return;
    };
    if !enabled || cfg!(debug_assertions) || test_run() {
        return;
    }
    let key = key.to_string();
    std::thread::spawn(move || {
        let first_today = first_today();
        let Ok(distinct_id) = crate::mail::random_hex(16) else {
            return;
        };
        let event = event(&key, &distinct_id, first_today, &macos_version());
        // Best effort: offline or refused, the launch is simply not counted.
        let _ = ureq::post(ENDPOINT)
            .timeout(Duration::from_secs(5))
            .send_json(event);
    });
}

/// The event sent, and all of it.
fn event(key: &str, distinct_id: &str, first_today: bool, macos: &str) -> serde_json::Value {
    serde_json::json!({
        "api_key": key,
        "event": "app_opened",
        "distinct_id": distinct_id,
        "properties": {
            "$process_person_profile": false,
            "$geoip_disable": true,
            "first_today": first_today,
            "app_version": env!("CARGO_PKG_VERSION"),
            "macos_version": macos,
        },
    })
}

/// A run made for testing or screenshots, which should not count.
fn test_run() -> bool {
    ["NIMBRA_DEMO", "NIMBRA_SCREEN"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
}

/// Where the day of the last launch is kept: a date, nothing else.
fn last_opened_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Nimbra/last-opened"))
}

/// Whether this is the first launch today on this Mac, noting today as seen.
fn first_today() -> bool {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let Some(path) = last_opened_path() else {
        return false;
    };
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    if before.trim() == today {
        return false;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &today);
    true
}

/// The macOS major version ("26"), or empty when it cannot be read.
fn macos_version() -> String {
    std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|v| v.trim().split('.').next().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_carries_nothing_but_the_counts() {
        let event = event("phc_test", "abc", true, "26");
        let properties = event["properties"].as_object().unwrap();
        let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "$geoip_disable",
                "$process_person_profile",
                "app_version",
                "first_today",
                "macos_version"
            ]
        );
        assert_eq!(event["properties"]["$process_person_profile"], false);
        assert_eq!(event["event"], "app_opened");
    }
}
