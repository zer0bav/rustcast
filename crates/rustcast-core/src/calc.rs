//! Inline calculator provider. Prefers `qalc -t` when installed; otherwise falls
//! back to a built-in evaluator ([`crate::eval`]) that also handles unit and
//! currency conversion, so the calculator works with no external tools.

use crate::config::which;
use crate::model::{Action, Item, Prev};
use crate::provider::{Provider, QueryCtx, Tab};
use std::process::Command;

pub struct CalcProvider {
    /// Whether `qalc` is on PATH (preferred backend).
    qalc: bool,
}

impl CalcProvider {
    pub fn new() -> Self {
        CalcProvider { qalc: which("qalc") }
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
        let raw = ctx.raw.trim();
        let has_prefix = raw.starts_with('=');
        let expr = raw.strip_prefix('=').unwrap_or(raw).trim();
        let looks_math = expr.chars().any(|c| c.is_ascii_digit())
            && expr.chars().any(|c| "+-*/^%(".contains(c));
        let looks_convert = expr.contains(" in ") || expr.contains(" to ");
        if expr.is_empty() || (!looks_math && !looks_convert && !has_prefix) {
            return Vec::new();
        }

        // Prefer qalc; fall back to the built-in evaluator (also does unit &
        // currency conversion).
        let res = self.qalc_eval(expr).or_else(|| crate::eval::evaluate(expr));
        let Some(res) = res else { return Vec::new() };
        if res.is_empty() || res == expr {
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

impl CalcProvider {
    fn qalc_eval(&self, expr: &str) -> Option<String> {
        if !self.qalc {
            return None;
        }
        let out = Command::new("qalc").arg("-t").arg(expr).output().ok()?;
        let res = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if res.is_empty() || res == expr || res.to_lowercase().contains("error") {
            return None;
        }
        Some(res)
    }
}
