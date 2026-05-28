# DevFlow Recorder

DevFlow Recorder is a local-first workflow recorder for Linux developers. It watches the active window on Wayland desktops, merges focus changes into a readable timeline, and stores the result locally so you can review what you worked on during the day.

The project is currently an MVP. It is useful for testing real desktop activity collection, especially on GNOME Wayland, but the reporting and browser/project integrations are still early.

## What It Does

- Records active application and window title changes.
- Aggregates repeated focus switches into activity segments instead of appending noisy duplicate rows.
- Restores today's timeline after restarting the app.
- Persists activity data in a local SQLite database.
- Enriches terminal activity with lightweight process hints such as `hermes x3`, `codex`, `node`, or `reasonix`.
- Provides a GNOME Shell extension bridge for GNOME Wayland, where ordinary desktop apps cannot directly read global window focus.
- Keeps the privacy boundary intentionally narrow: no screenshots, no keyboard logging, no terminal command arguments.

## Why This Exists

Most activity trackers are either too invasive, too cloud-oriented, or not designed for Wayland's security model. DevFlow Recorder explores a different path:

- Local data first.
- Explicit desktop providers instead of fragile global hacks.
- Developer-oriented context, especially terminals, editors, browsers, and project windows.
- A timeline that is useful for daily review rather than a raw stream of focus events.

The long-term goal is to turn local activity into a concise Chinese daily report and a searchable development memory.

## Current Architecture

- Frontend: React + Vite
- Desktop shell: Tauri 2
- Backend: Rust
- Storage: SQLite via `rusqlite`
- GNOME integration: GNOME Shell extension + local HTTP bridge
- Hyprland provider: `hyprctl activewindow -j`
- Sway provider: `swaymsg -t get_tree`

The Tauri backend owns the timeline state, local bridge, persistence, and provider polling. The frontend renders snapshots and can pause or resume recording.

## Provider Support

| Desktop | Status | How It Works |
| --- | --- | --- |
| GNOME / Mutter | Working MVP | GNOME Shell extension reports focused window metadata to the local bridge. |
| Hyprland | Basic support | Backend polls `hyprctl activewindow -j`. |
| Sway | Basic support | Backend scans `swaymsg -t get_tree` for the focused node. |
| KDE / KWin | Planned | Needs an explicit KWin script or DBus provider. |
| Browser tabs | Planned | A browser extension bridge is reserved but not implemented yet. |

Wayland intentionally prevents ordinary clients from globally inspecting other windows. DevFlow Recorder treats each desktop environment as a separate provider instead of trying to bypass that model.

## Privacy Model

DevFlow Recorder is designed to be open-source friendly and privacy-conscious.

It currently records:

- Window title
- Application name / app id / wm class
- Window pid when provided by the compositor or GNOME Shell
- Workspace id when provided by GNOME Shell
- Activity timestamps and segment durations
- Terminal child process names, `argv0` basename, and cwd for lightweight context

It does not record:

- Screenshots
- Keyboard input
- Mouse input
- Clipboard contents
- Terminal command arguments
- Terminal environment variables
- File contents
- Browser URLs from the system layer

Browser URL collection, when implemented, should be opt-in through a browser extension and should strip query strings and fragments by default.

## Local Data

Runtime data is stored under:

```text
~/.local/share/devflow-recorder/
```

Important files:

- `devflow-recorder.sqlite`: local activity database
- `devflow-recorder.sqlite-wal`: SQLite WAL file
- `devflow-recorder.sqlite-shm`: SQLite shared memory file
- `bridge-token`: local token used by the GNOME Shell extension when posting to the bridge

These files are runtime data and should not be committed to Git.

## Install Dependencies

You need Node.js, npm, Rust, Cargo, and the native dependencies required by Tauri 2 on Linux.

Install JavaScript dependencies:

```bash
npm install
```

Build the frontend:

```bash
npm run build
```

Run the desktop app in development mode:

```bash
npm run tauri:dev
```

## GNOME Shell Extension

GNOME Wayland needs the included Shell extension to expose focused window metadata.

Install the extension:

```bash
./tools/install-gnome-extension.sh
```

Enable it:

```bash
gnome-extensions enable devflow-recorder@local
```

If GNOME says the extension was not found, log out and back in on Wayland, then enable it again.

When enabled, the top bar shows a small `DF` indicator. The extension reports focused window metadata to the local bridge:

```text
http://127.0.0.1:45173/v1/gnome/window
```

The backend generates a local `bridge-token` on startup. The extension reads that token and sends it with the `X-DevFlow-Token` header. Requests without the token are rejected.

## Development Commands

```bash
npm install
npm run build
npm run tauri:dev
```

Rust checks:

```bash
cd src-tauri
cargo check
cargo clippy --all-targets --all-features
```

## Project Layout

```text
.
├── gnome-extension/              # GNOME Shell extension provider
├── src/                          # React frontend
├── src-tauri/                    # Rust / Tauri backend
├── tools/                        # Local helper scripts
├── README.md
└── package.json
```

## Roadmap

- Browser extension for active tab title and sanitized URL.
- KDE / KWin provider.
- Better project detection from Git repositories, editor windows, and terminal cwd.
- Settings page for retention, provider toggles, export, and clearing local data.
- Daily report generation from local timeline data.
- Packaging for easier Linux installation.

## License

MIT
