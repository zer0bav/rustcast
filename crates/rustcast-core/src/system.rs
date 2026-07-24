//! System commands (power, audio, network) and compositor window management.
//! Every command is gated on the presence of the tool it needs, so unavailable
//! ones simply don't appear. Lives in the Apps (root) tab.

use crate::config::which;
use crate::model::{Action, Item};
use crate::provider::{Provider, QueryCtx, Tab};

struct Cmd {
    title: &'static str,
    subtitle: &'static str,
    icon: &'static str,
    cmd: String,
}

pub struct SystemProvider {
    cmds: Vec<Cmd>,
}

impl SystemProvider {
    pub fn new() -> Self {
        let mut c = Vec::new();
        let mut add = |title, subtitle, icon, cmd: String| {
            c.push(Cmd { title, subtitle, icon, cmd });
        };

        // ── power / session ──
        let lock = if which("hyprlock") {
            "hyprlock"
        } else if which("swaylock") {
            "swaylock"
        } else {
            "loginctl lock-session"
        };
        add("Lock Screen", "lock the session", "system-lock-screen", lock.into());
        if which("systemctl") {
            add("Sleep", "suspend the system", "system-suspend", "systemctl suspend".into());
            add("Reboot", "restart the computer", "system-reboot", "systemctl reboot".into());
            add("Power Off", "shut down", "system-shutdown", "systemctl poweroff".into());
        }
        // logout (compositor-specific)
        if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
            add("Log Out", "exit Hyprland", "system-log-out", "hyprctl dispatch exit".into());
        } else if std::env::var("SWAYSOCK").is_ok() {
            add("Log Out", "exit Sway", "system-log-out", "swaymsg exit".into());
        }

        // ── audio (wireplumber) ──
        if which("wpctl") {
            add("Volume Up", "+5%", "audio-volume-high", "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+".into());
            add("Volume Down", "-5%", "audio-volume-low", "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-".into());
            add("Mute", "toggle output mute", "audio-volume-muted", "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle".into());
            add("Mic Mute", "toggle input mute", "microphone-sensitivity-muted", "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle".into());
        } else if which("pactl") {
            add("Volume Up", "+5%", "audio-volume-high", "pactl set-sink-volume @DEFAULT_SINK@ +5%".into());
            add("Volume Down", "-5%", "audio-volume-low", "pactl set-sink-volume @DEFAULT_SINK@ -5%".into());
            add("Mute", "toggle output mute", "audio-volume-muted", "pactl set-sink-mute @DEFAULT_SINK@ toggle".into());
        }

        // ── brightness ──
        if which("brightnessctl") {
            add("Brightness Up", "+10%", "display-brightness", "brightnessctl set +10%".into());
            add("Brightness Down", "-10%", "display-brightness", "brightnessctl set 10%-".into());
        }

        // ── network / radios ──
        if which("nmcli") {
            add("Wi-Fi On", "enable Wi-Fi radio", "network-wireless", "nmcli radio wifi on".into());
            add("Wi-Fi Off", "disable Wi-Fi radio", "network-wireless-offline", "nmcli radio wifi off".into());
        }
        if which("rfkill") {
            add("Bluetooth On", "unblock bluetooth", "bluetooth", "rfkill unblock bluetooth".into());
            add("Bluetooth Off", "block bluetooth", "bluetooth-disabled", "rfkill block bluetooth".into());
        }
        if which("gio") {
            add("Empty Trash", "delete trashed files", "user-trash", "gio trash --empty".into());
        }

        // ── window management (compositor-detected) ──
        if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
            add("Toggle Fullscreen", "active window", "view-fullscreen", "hyprctl dispatch fullscreen 1".into());
            add("Toggle Floating", "active window", "window-new", "hyprctl dispatch togglefloating".into());
            add("Center Window", "active window", "window", "hyprctl dispatch centerwindow".into());
            add("Close Window", "kill active window", "window-close", "hyprctl dispatch killactive".into());
            add("Next Workspace", "switch workspace", "go-next", "hyprctl dispatch workspace e+1".into());
            add("Previous Workspace", "switch workspace", "go-previous", "hyprctl dispatch workspace e-1".into());
        } else if std::env::var("SWAYSOCK").is_ok() {
            add("Toggle Fullscreen", "active window", "view-fullscreen", "swaymsg fullscreen toggle".into());
            add("Toggle Floating", "active window", "window-new", "swaymsg floating toggle".into());
            add("Close Window", "kill active window", "window-close", "swaymsg kill".into());
        }

        SystemProvider { cmds: c }
    }
}

impl Default for SystemProvider {
    fn default() -> Self {
        SystemProvider::new()
    }
}

impl Provider for SystemProvider {
    fn id(&self) -> &'static str {
        "system"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim();
        if q.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for c in &self.cmds {
            if let Some(s) = crate::ranking::score(ctx.matcher, c.title, c.subtitle, q) {
                out.push(
                    Item::new(
                        c.title,
                        c.subtitle,
                        c.icon,
                        "system",
                        s,
                        Action::RunShell(c.cmd.clone()),
                    )
                    .in_section(crate::registry::section::SYSTEM),
                );
            }
        }
        out
    }
}
