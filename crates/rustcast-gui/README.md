# rustcast

A Raycast-class launcher for Linux — GTK4 + `wlr-layer-shell`, with a resident
daemon for instant open, sectioned results ranked by match quality and your
usage history, a general **tldr** command search, native clipboard history and
file search.

This crate installs the `rustcast` binary.

```sh
cargo install rustcast-linux
```

You need GTK 4 (and, for the wlroots overlay, `gtk4-layer-shell`) installed.
Then bind `rustcast` to a global hotkey — it toggles the window.

## Highlights

- **Instant open** — resident daemon; the hotkey toggles the window in ~50 ms.
- **tldr search** — 10k+ community command examples; Enter copies one command.
- **Tiered ranking** — exact › prefix › word-start › initials › substring ›
  fuzzy, with **frecency** breaking ties; results grouped under section headers.
- **Web-search fallback**, config **aliases**.
- **Built-in calculator + unit/currency converter** (works without `qalc`).
- **Clipboard history** (text + images) with a rich preview, in-list delete/pin
  (`Ctrl+D` / `Ctrl+S`) and OCR; **file search** with drag-out.
- Kill-process, port-inspector, window-switcher, quicklinks, pins, script plugins.

See the [project README](https://github.com/zer0bav/rustcast) for full docs,
configuration, and per-compositor hotkey snippets.

## License

MIT
