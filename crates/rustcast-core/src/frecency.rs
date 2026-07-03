//! Frecency: a usage + recency score that floats your most-used items to the
//! top of the Apps root, the way Raycast does.
//!
//! Keyed by [`crate::pins::pin_key`] (the same stable identity used for pins), so
//! only launchable actions (apps, quicklinks, command modes, files) are tracked.
//! Persisted as a small JSON map at `~/.local/share/rustcast/frecency.json`.

use crate::config::Config;
use std::collections::HashMap;

/// How many usages to keep before pruning the least valuable.
const MAX_ENTRIES: usize = 400;
/// Recency half-life in days: a use "counts half" after this long.
const HALF_LIFE_DAYS: f64 = 7.0;
/// Ceiling on the boost so a habitual app can outrank a marginal fuzzy match but
/// never displaces pins (20_000), quicklinks (6_000) or command hits (~400+).
const MAX_BOOST: i64 = 250;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub count: u32,
    /// Unix seconds of the last use.
    pub last: i64,
}

#[derive(Default)]
pub struct Frecency {
    map: HashMap<String, Entry>,
}

/// Current wall-clock in unix seconds (single place, so callers stay testable).
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn path() -> Option<std::path::PathBuf> {
    Config::data_dir().map(|d| d.join("frecency.json"))
}

impl Frecency {
    pub fn new() -> Self {
        Frecency { map: HashMap::new() }
    }

    /// Load from disk (empty on any error).
    pub fn load() -> Self {
        let Some(p) = path() else { return Frecency::new() };
        let Ok(text) = std::fs::read_to_string(p) else { return Frecency::new() };
        let map = serde_json::from_str(&text).unwrap_or_default();
        Frecency { map }
    }

    /// Persist to disk.
    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&self.map) {
            let _ = std::fs::write(p, text);
        }
    }

    /// Record one use of `key` now, pruning the map if it has grown too large.
    pub fn record(&mut self, key: &str) {
        let now = now_unix();
        let e = self.map.entry(key.to_string()).or_insert(Entry { count: 0, last: now });
        e.count = e.count.saturating_add(1);
        e.last = now;
        if self.map.len() > MAX_ENTRIES {
            self.prune(now);
        }
    }

    /// Drop the lowest-boost entries back down to `MAX_ENTRIES`.
    fn prune(&mut self, now: i64) {
        let mut scored: Vec<(String, i64)> =
            self.map.iter().map(|(k, e)| (k.clone(), boost_of(e, now))).collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        let keep: std::collections::HashSet<String> =
            scored.into_iter().take(MAX_ENTRIES).map(|(k, _)| k).collect();
        self.map.retain(|k, _| keep.contains(k));
    }

    /// Score boost for `key` (0 if never used).
    pub fn boost(&self, key: &str, now: i64) -> i64 {
        self.map.get(key).map(|e| boost_of(e, now)).unwrap_or(0)
    }
}

/// `ln(1+count) * 60 * 0.5^(age_days / half_life)`, capped. Frequent + recent
/// wins; an old-but-heavy item still gets a small nudge.
fn boost_of(e: &Entry, now: i64) -> i64 {
    let age_days = ((now - e.last).max(0) as f64) / 86_400.0;
    let weight = 0.5_f64.powf(age_days / HALF_LIFE_DAYS);
    let raw = (1.0 + e.count as f64).ln() * 60.0 * weight;
    (raw as i64).min(MAX_BOOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boost_decays_by_half_after_one_half_life() {
        let now = 1_000_000_000;
        let fresh = Entry { count: 10, last: now };
        let old = Entry { count: 10, last: now - (HALF_LIFE_DAYS as i64) * 86_400 };
        let bf = boost_of(&fresh, now);
        let bo = boost_of(&old, now);
        assert!(bf > 0);
        // within rounding, the older one is ~half
        assert!((bo as f64 - bf as f64 / 2.0).abs() <= 2.0, "fresh={bf} old={bo}");
    }

    #[test]
    fn boost_is_capped() {
        let now = 2_000_000_000;
        let heavy = Entry { count: 100_000, last: now };
        assert_eq!(boost_of(&heavy, now), MAX_BOOST);
    }

    #[test]
    fn record_and_boost_roundtrip() {
        let mut f = Frecency::new();
        f.record("launch:firefox");
        f.record("launch:firefox");
        let now = now_unix();
        assert!(f.boost("launch:firefox", now) > 0);
        assert_eq!(f.boost("launch:never", now), 0);
    }

    #[test]
    fn prune_keeps_the_most_valuable() {
        let mut f = Frecency::new();
        let now = now_unix();
        for i in 0..(MAX_ENTRIES + 50) {
            // give lower indices more uses so they should survive
            let uses = (MAX_ENTRIES + 50 - i) as u32;
            f.map.insert(format!("launch:app{i}"), Entry { count: uses, last: now });
        }
        f.prune(now);
        assert!(f.map.len() <= MAX_ENTRIES);
        assert!(f.map.contains_key("launch:app0"));
    }
}
