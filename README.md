# Linux Mod Manager

A native Linux application for managing game mods. Built with [Tauri v2](https://tauri.app) —  Rust backend, HTML/CSS/JS frontend rendered in the system WebKit webview (no bundled browser).

## Features

- **NexusMods Integration**: Browse, search, and download mods directly from NexusMods
- **Mod Management**: Enable/disable, uninstall, and manage mod lifecycle
- **Cyberpunk 2077 Support**: Game-specific mod handling and component management
- **Deep Linking**: Support for NXM protocol and single-instance application handling
- **Multi-Game Support**: Extensible architecture for supporting multiple games
- **Steam Library Detection**: Automatic detection of Steam libraries

## Install / Run

```bash
git clone https://github.com/ATack-45/Linux-Mod-Manager.git && cd Linux-Mod-Manager && bash install.sh
```

`install.sh` handles everything automatically:
1. Installs Rust via rustup (if not present)
2. Installs WebKitGTK via your distro's package manager
3. Installs `tauri-cli` via Cargo
4. Builds and launches the app in dev mode

> First run compiles all Rust dependencies — takes several minutes. Every run after that is fast.

## Development

```bash
cargo tauri dev
```

Frontend files in `frontend/` hot-reload on save. Rust changes in `src-tauri/` trigger a recompile.

## Build distributable packages

```bash
cargo tauri build
```

Outputs `.deb`, `.rpm`, and `AppImage` to `src-tauri/target/release/bundle/`.

## Project structure

```
frontend/              # UI — edit these for layout and style
  index.html
  style.css
  main.js
src-tauri/             # Rust/Tauri backend
  src/main.rs          # App entry point (thin shell for now)
  src/modding/         # Mod management and scanning logic
  src/nexusmods.rs     # NexusMods API integration
  tauri.conf.json      # Window config, icon paths, bundle settings
  Cargo.toml           # Rust dependencies
  build.rs             # Required Tauri build script
  icons/               # App icons (32px – 512px)
install.sh             # One-command setup script
```

> `src-tauri/` and `src-tauri/src/` names are required by the Tauri CLI and Cargo respectively — they can't be renamed.

## Requirements

- Linux (Debian/Ubuntu, Fedora, Arch, openSUSE — detected automatically by `install.sh`)
- Rust + Cargo (installed by `install.sh`)
- WebKitGTK 4.1 (installed by `install.sh`)
