//! Root command palette — browsable entries that "enter" a tool mode.
//!
//! Raycast-style: instead of memorising the `kill`/`win`/`gen`/`port` prefixes,
//! you find "Kill Process", "Window Switcher", etc. in the root list and press
//! Enter to go inside. Activating one sets the search box to that prefix
//! ([`Action::SetQuery`]), so the matching provider takes over and you filter by
//! typing. Esc clears the box and backs you out.

use crate::model::{Action, Item};
use crate::provider::{Provider, QueryCtx, Tab};

struct Command {
    title: &'static str,
    subtitle: &'static str,
    icon: &'static str,
    /// Extra words to match on (so "task manager" finds "Kill Process").
    keywords: &'static str,
    /// Provider id of the mode this command enters.
    mode: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        title: "Kill Process",
        subtitle: "browse running processes and terminate them",
        icon: "utilities-system-monitor",
        keywords: "kill process task manager terminate stop end proc",
        mode: "procs",
    },
    Command {
        title: "Window Switcher",
        subtitle: "jump to any open window",
        icon: "window",
        keywords: "window switcher switch alt tab move focus",
        mode: "windows",
    },
    Command {
        title: "Port Inspector",
        subtitle: "find and kill whatever is listening on a port",
        icon: "network-server",
        keywords: "port listen socket netstat ss lsof tcp udp",
        mode: "ports",
    },
    Command {
        title: "Generate Secret",
        subtitle: "passwords, hex/base64 tokens, UUIDs, PINs",
        icon: "dialog-password",
        keywords: "generate password token secret uuid pin random gen",
        mode: "gen",
    },
    Command {
        title: "Search tldr",
        subtitle: "10k+ community command examples (tar, ssh, curl…)",
        icon: "utilities-terminal",
        keywords: "tldr man help example command reference cli how do i",
        mode: "tldr",
    },
    Command {
        title: "Search Cheatsheets",
        subtitle: "command references (nmap, tmux, vim, curl…)",
        icon: "accessories-dictionary",
        keywords: "cheat cheatsheet reference help docs",
        mode: "cheatsheets",
    },
    Command {
        title: "Add Quicklink",
        subtitle: "create a URL/command shortcut",
        icon: "list-add",
        keywords: "add quicklink shortcut url bookmark new create",
        mode: "add-quicklink",
    },
];

pub struct CommandsProvider;

impl CommandsProvider {
    pub fn new() -> Self {
        CommandsProvider
    }
}

impl Default for CommandsProvider {
    fn default() -> Self {
        CommandsProvider::new()
    }
}

impl Provider for CommandsProvider {
    fn id(&self) -> &'static str {
        "commands"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim();
        let mut out = Vec::new();
        for c in COMMANDS {
            // Same tiers as apps, so "kill" hits Kill Process on the prefix tier
            // and "task manager" still finds it through the keyword tier.
            let Some(score) = crate::ranking::score(ctx.matcher, c.title, c.keywords, q) else {
                continue;
            };
            // On the empty root, commands sit just under the apps baseline: the
            // launcher is for launching first, browsing tools second.
            let score = if q.is_empty() { crate::ranking::IDLE - 100 } else { score };
            out.push(
                Item::new(
                    c.title,
                    c.subtitle,
                    c.icon,
                    "command",
                    score,
                    Action::EnterMode { id: c.mode.to_string(), label: c.title.to_string() },
                )
                .in_section(crate::registry::section::COMMANDS),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_match_finds_command() {
        let p = CommandsProvider::new();
        let m = crate::ranking::matcher();
        let ctx = QueryCtx { raw: "task manager", query: "task manager", active_tab: Tab::Apps, matcher: &m, mode: None };
        let r = p.query(&ctx);
        assert!(r.iter().any(|i| i.title == "Kill Process"));
    }

    #[test]
    fn empty_shows_all_commands() {
        let p = CommandsProvider::new();
        let m = crate::ranking::matcher();
        let ctx = QueryCtx { raw: "", query: "", active_tab: Tab::Apps, matcher: &m, mode: None };
        assert_eq!(p.query(&ctx).len(), COMMANDS.len());
    }
}
