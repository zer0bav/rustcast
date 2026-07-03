//! Inline calculator provider. Shells to `qalc -t`. Kept synchronous for now
//! (matches the prototype); a debounced async path is a later refinement.

use crate::config::which;
use crate::model::{Action, Item, Prev};
use crate::provider::{Provider, QueryCtx, Tab};
use std::process::Command;

pub struct CalcProvider {
    available: bool,
}

impl CalcProvider {
    pub fn new() -> Self {
        CalcProvider { available: which("qalc") }
    }
}

impl Default for CalcProvider {
    fn default() -> Self {
        CalcProvider::new()
    }
}

impl Provider for CalcProvider {
    fn id(&self) -> &'static str {
        "calc"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn prefix(&self) -> Option<&'static str> {
        Some("=")
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        if !self.available {
            return Vec::new();
        }
        let raw = ctx.raw.trim();
        let expr = raw.strip_prefix('=').unwrap_or(raw).trim();
        let looks_math = expr.chars().any(|c| c.is_ascii_digit())
            && expr.chars().any(|c| "+-*/^%(".contains(c));
        if expr.is_empty() || (!looks_math && !raw.starts_with('=')) {
            return Vec::new();
        }
        let Ok(out) = Command::new("qalc").arg("-t").arg(expr).output() else {
            return Vec::new();
        };
        let res = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if res.is_empty() || res == expr || res.to_lowercase().contains("error") {
            return Vec::new();
        }
        vec![Item::new(
            format!("= {res}"),
            format!("{expr}  ·  copy to clipboard"),
            "accessories-calculator",
            "calc",
            10_000, // pin to top
            Action::Copy(res.clone()),
        )
        .with_prev(Prev::Text(res))]
    }
}
