<p align="center">
  <img src="eave.png" alt="Eave icon" width="96" height="96" />
</p>

<h1 align="center">Eave</h1>

<p align="center">
  <strong>A native, ultra-lightweight Dynamic Island companion for Windows.</strong><br />
  Built in pure Rust with Win32, DirectComposition, and Direct2D — zero Electron, zero WebView, ~0% idle CPU.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011-0078D4?style=flat-square&logo=windows" alt="Windows 10/11" />
  <img src="https://img.shields.io/badge/Rust-Pure%20Native-DEA584?style=flat-square&logo=rust" alt="Pure Rust" />
  <img src="https://img.shields.io/badge/Renderer-Direct2D%20%2B%20DComp-111111?style=flat-square" alt="Direct2D + DirectComposition" />
  <img src="https://img.shields.io/badge/License-MIT-22C55E?style=flat-square" alt="MIT License" />
</p>

---

## Features

- **Now Playing & Synced Lyrics** — Connects to Windows SMTC (Spotify, Apple Music, browsers, etc.) with adaptive album-art glow, interactive playback controls, progress scrubbing, and live synced lyrics.
- **Eave AI** — Open an inline AI assistant from anywhere on Windows (`Alt + Q`) with streaming responses, Markdown & LaTeX rendering, clickable links, text selection, and persistent 10-turn history.
- **Native Notifications & System OSDs** — Intercepts Windows Toast notifications with **real app icons and sender avatars**, plus smooth island indicators for Volume, Brightness, Caps Lock, Microphone, Charging, Wi-Fi, Bluetooth, and Audio Output switching.
- **To-do & Scheduled Reminders** — Quick checklist right inside the notch with optional `HH:MM–HH:MM` schedules, `Daily / Once` repeat, chronological schedule filter, and interactive notch alerts at start and end times.
- **Shelf, Clipboard & Smart OCR** — Drag-and-drop file tray with native Windows shell icons, searchable clipboard history, and offline screen text extraction (`Ctrl + Alt + X`) powered by Windows OCR.
- **Focus & Hydration** — Built-in Pomodoro timer with round timeline and interactive break alerts, plus a Water intake tracker with an hourly chart and circular hydration gauge.
- **Customizable Widgets** — Mix and match Weather, Battery, World Clock, Weekly Calendar, Countdown, and Quick Controls.
- **Built for Speed** — Spring-based physics animations, on-demand rendering (0% idle CPU), automatic fullscreen hiding for games and video, and a native dark Settings UI with automatic GitHub Releases update checks.

---

## Default Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Alt + Q` | Open / close **Eave AI** (customizable in Settings) |
| `Ctrl + Alt + X` | Launch **Smart OCR** to copy text from any screen region |
| `Ctrl + Alt + H` | Toggle **Hide in full screen** from any app or game |
| `Hover / Click` | Expand or collapse the notch |
| `Scroll Wheel` | Switch pages inside the expanded notch |

---

## Download & Installation

1. Download the latest **`eave.exe`** from the [Releases](../../releases/latest) page.
2. Run `eave.exe` — no installer required.
3. Right-click the **Eave** tray icon (or open the dock gear) to customize pages, shortcuts, appearance, and **Start with Windows**.

> **Auto-Updates:** Eave automatically checks GitHub Releases on startup (configurable under **Settings → About**).

---

## Build from Source

### Requirements
- Windows 10 (1903+) or Windows 11
- [Rust toolchain](https://rustup.rs/) (MSVC target)

### Compile & Run
```powershell
git clone https://github.com/xvly0/eave.git
cd eave
cargo build --release
.\target\release\eave.exe
```

Run the test suite:
```powershell
cargo test
```

---

## Configuration

Settings and local data are stored in `%APPDATA%\eave\`:

- `eave.cfg` — All preferences (also supports portable mode if placed next to `eave.exe`)
- `tasks.txt` — To-do items and schedules
- `shelf.txt` — Pinned shelf files
- `ai_history.json` — Recent Eave AI conversation turns

---

## License

Released under the [MIT License](LICENSE).
