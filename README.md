# rustcast 🦀🚀

A **Raycast-class application launcher for Linux**, written in Rust — fast, native,
keyboard-driven, with a built-in **cybersecurity toolkit**.

> Linux deserves a launcher as good as Raycast: instant, extensible, hacker-red —
> and useful whether you're red team, blue team, or just launching apps.

![Command palette](docs/screenshots/apps.png)

## Highlights

- ⚡ **App launcher** — fuzzy search over your `.desktop` apps, with icons
- 🎛️ **Command palette** — the root list mixes your apps with commands like
  "Kill Process", "Window Switcher", "Generate Secret"; press Enter to drop into
  an **isolated mode** where typing just filters (Esc backs out). No magic
  prefixes, so a command never collides with an app of the same name
- 📋 **Native clipboard history** — text **and** images, live previews, pin/delete,
  dedup. Its own background watcher (no `cliphist`, no prefixes)
- 🔎 **File search** — fuzzy find with live preview and **drag-and-drop** out to
  other apps (plus copy-path fallback)
- 🛡️ **Cyber toolkit** — base64/hex/url/rot13 codecs, md5/sha1/sha256/sha512,
  JWT decode, CIDR calculator, epoch↔time, defang/refang, reverse-shell generator,
  and OSINT link-outs (VirusTotal/Shodan/NVD…) — all live as you type
- 💀 **Kill Process** — lists processes with owner + memory; Enter sends SIGTERM,
  Ctrl+K force-kills; stays open so you can kill several in a row
- 🔌 **Port Inspector** — shows what's listening on a port and kills it
- 🪟 **Window Switcher** — its own tab (or `Super+Alt+W`); Enter focuses a window
- 🔐 **Generate Secret** — strong passwords, hex/base64 tokens, UUIDs, PINs
- 📖 **Cheatsheets** — a browsable tab of Markdown-rendered command references
  (nmap, tmux, vim, curl, gdb, hashcat, sqlmap, linux-privesc…) plus your own
  `~/.config/rustcast/cheatsheets/*.md`
- 🔗 **Quicklinks** — `{query}` URL/command templates, addable from the UI
  ("Add Quicklink"), no config editing required
- 🧩 **Extensions** — script plugins in any language (JSON over stdout)
- 🧮 **Calculator**, ✂️ **snippets**, 🖥️ **system + window-management commands**
- ⌨️ **Mode tabs** (Apps · Clipboard · Files · Cyber · Cheats · Windows · Extensions)
  and a Cmd-K style actions menu
- 🔴⚫ Softened red/black theme, English UI, GTK4

### More screenshots

| Cyber toolkit | Cheatsheets (Markdown) |
|---|---|
| ![Cyber](docs/screenshots/cyber.png) | ![Cheats](docs/screenshots/cheats.png) |
| **File search + drag-out** | **Window switcher** |
| ![Files](docs/screenshots/files.png) | ![Windows](docs/screenshots/windows.png) |
| **Extensions & settings** | |
| ![Extensions](docs/screenshots/extensions.png) | |

## Works everywhere

- **Hyprland / Sway / river / wayfire** — renders as a `wlr-layer-shell` overlay.
- **GNOME / KDE / any Wayland or X11 desktop** — automatically falls back to a
  normal borderless window that closes on focus loss.
- Clipboard/paste uses `wl-clipboard` on Wayland and `xclip`/`xsel` on X11.
  Clipboard **history** needs a Wayland session (`wl-paste`).

## Install

```bash
git clone https://github.com/zer0bav/rustcast
cd rustcast
./install.sh
```

`install.sh` builds the release binary, installs it to `~/.local/bin`, adds a
desktop entry and default config, and (on systemd + Wayland) enables the clipboard
history daemon. It prints the exact keybinding snippet for your desktop.

Build dependencies: `cargo`, `gtk4` (dev), and — for the overlay mode on
wlroots — `gtk4-layer-shell` (dev).

```
Arch:          sudo pacman -S gtk4 gtk4-layer-shell
Debian/Ubuntu: sudo apt install libgtk-4-dev libgtk4-layer-shell-dev
Fedora:        sudo dnf install gtk4-devel gtk4-layer-shell-devel
```

## Bind a hotkey

rustcast doesn't grab a global hotkey itself (that's the compositor's job):

```ini
# Hyprland (~/.config/hypr/…)
bind = SUPER, SPACE, exec, rustcast
bind = SUPER, V,     exec, rustcast --tab clipboard
```
```ini
# Sway (~/.config/sway/config)
bindsym $mod+space exec rustcast
bindsym $mod+v     exec rustcast --tab clipboard
```
- **GNOME:** Settings → Keyboard → Custom Shortcuts → command `rustcast`
- **KDE:** System Settings → Shortcuts → Custom → command `rustcast`

## Usage

- Type to search; `↑`/`↓` (or `Ctrl+J/K`) to move; `Enter` to run.
- `Tab` / `Shift+Tab` cycle tabs; `Ctrl+1..7` jump to one.
- `Ctrl+K` opens the actions menu (copy, delete, pin, reveal, force-kill, …).
- `Esc` clears the query, then closes.

**Cyber tab** — type a keyword or just paste: `b64 hello`, `hash secret`, a JWT,
`cidr 10.0.0.0/24`, `ts 1516239022`, `rev 10.0.0.5:4444`, `defang http://1.2.3.4`,
`link CVE-2021-44228`, `target 10.0.0.5`.

**Command modes** — find a command in the root list ("Kill Process", "Window
Switcher", "Port Inspector", "Generate Secret", "Search Cheatsheets") and press
Enter to drop into an isolated view where typing just filters within it (Esc
backs out). Because you *enter* the mode rather than type a magic prefix, a
command never collides with an app of the same name. The **Windows** tab (or
`Super+Alt+W`) opens the window switcher directly.

The cyber toolkit still uses inline prefixes (`= 2+2`, `b64 hello`, `hash …`,
`jwt …`, `cidr …`, `rev host:port`, `link CVE-…`).

**Cheats tab** — browse bundled command references, or drop your own Markdown
into `~/.config/rustcast/cheatsheets/`. Enter copies the whole sheet; the preview
pane shows it as you scroll.

## Configuration

Config lives at `~/.config/rustcast/config.toml` (see `config.example.toml`):
UI size, terminal, clipboard cap, file roots, **quicklinks**, and **snippets**.
Drop a custom theme at `~/.config/rustcast/style.css`.

## Extensions

Create `~/.config/rustcast/plugins/<name>/manifest.toml`
(`name`, `prefix`, `icon`, `exec`). The executable gets the query as `$1` and
prints:

```json
{ "items": [
  { "title": "…", "subtitle": "…", "icon": "…",
    "action": { "kind": "copy|open|shell|launch", "data": "…" } }
] }
```

See `plugins/example-echo/` for a working sample.

## Architecture

A Cargo workspace: **`rustcast-core`** (pure logic, no GTK — providers, ranking,
config, and the cyber toolkit, all unit-tested headlessly) and **`rustcast-gui`**
(the GTK4 binary). Providers implement a common trait and plug into tabs and
inline prefixes via a registry.

```bash
cargo test -p rustcast-core   # headless unit tests
cargo build --release
```

## License

[MIT](LICENSE) © 2026 zer0bav
