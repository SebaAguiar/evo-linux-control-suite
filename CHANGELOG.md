# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Releases and changelog entries follow [Conventional Commits](https://www.conventionalcommits.org/).

---

## [1.0.1] - 2026-10-26

### Fixed

- **Auto-updater:** the release workflow now builds the AppImage alongside the `.deb` and publishes a signed `latest.json`, so in-app updates are actually delivered. The manifest only advertises the formats the Tauri plugin can install (`linux-x86_64-appimage`, `-deb`, `-rpm`), verifies every signature, and fails loudly instead of silently skipping the manifest.
- **Remote QR / LAN detection:** `/api/info` now sends CORS headers for accepted origins, so the desktop app no longer reports "no se detectó una IP de red Wi-Fi/LAN activa" when the LAN listener is up. The QR modal distinguishes "no LAN IP", "remote server disabled" and "daemon unreachable".
- **Arch/CachyOS installer:** dependencies are skipped when already satisfied, HTML error pages cached as packages are purged, and a stale pacman database now offers a full `pacman -Syu` before retrying. Added `install-arch.sh` for the native package path.

## [1.0.0] - 2026-10-19

First official stable release of the **EVO Linux Control Suite** adhering to Semantic Versioning (SemVer).

### Added

#### Auto-Updater & In-App Upgrades
- In-app update checker integrated into the desktop application (`useUpdater` hook) checking against GitHub Releases every hour.
- One-click update button in the launcher header (`TitleBar`) that displays live download percentage progress and automatically relaunches the application upon completion.
- Ed25519 cryptographic signing of releases and Debian (`.deb`) bundles.
- Automatic `latest.json` static manifest generator (`generate-latest-json.mjs`) for frictionless OTA updates.

#### Open Source Infrastructure & Release CI
- Automated GitHub Actions workflow for release creation on git tag (`v*`) with asset packaging and signature verification.
- GitHub Issue Forms for bug reports and feature requests, Pull Request template with testing checklist, and Dependabot dependency management.
- Comprehensive `README.md` with system architecture diagrams, installation instructions, and hardware compatibility matrix.

#### Core Architecture & System Daemon (`crates/`)
- Hexagonal architecture decoupling domain logic from hardware transport:
  - `crates/evo-domain`: Pure audio state models, channel entities, monitor mixes, limits, and domain validations.
  - `crates/evo-ports`: Inbound/outbound interfaces for hardware controllers, persistence, and state observers.
  - `crates/evo-core`: Application services orchestrating gains, mutes, phantom power (48V), monitor balance, and SmartGain.
  - `crates/evo-mock`: Deterministic hardware simulator mimicking Audient EVO USB HID behavior for testing without physical hardware.
  - `crates/evo-persist`: Fast disk state persistence (`~/.config/evo-suite/state.json`) for seamless restore on boot.
  - `crates/evo-ipc`: Asynchronous inter-process protocol and state serialization.
  - `crates/evo-remote`: Low-latency WebSocket server (`ws://127.0.0.1:8080/ws`) and HTTP API (`/healthz`, `/api/info`).
  - `crates/evo-daemon`: Lightweight background system service (systemd user unit) owning USB HID / ALSA control.

#### Native Desktop Console (`apps/desktop/`)
- High-performance native GUI built with **React 19**, **TypeScript**, and **Tauri v2**:
  - **Horizontal 3-Wing Console Layout**: Replicates the physical desktop hardware form factor of Audient EVO interfaces.
  - **Inputs Wing**: Individual Channel 1 & Channel 2 strips with circular rotary gauges, independent 48V phantom power with safety cooldown lock, per-channel mute, and prominent SmartGain action button.
  - **Center Dial Wing**: Multifunction rotary encoder with 24-LED white halo ring, smooth drag interaction, direct numeric dB entry (via double-click), and quick steppers (`-2 dB`, `-0.5`, `+0.5`, `+2 dB`).
  - **Outputs Wing**: Speaker volume control, DAW ↔ Hardware monitoring mix balance, master mute with partial mute indicator, and status badge.
  - **System Tray Integration**: Native tray icon with quick actions (Show/Hide window, Mute All, Quit).
  - **QR Code Modal**: Quick pairing modal to access the remote web console from mobile devices on the same local network.

#### Remote Mobile Web App (`apps/remote-web/`)
- Zero-dependency, responsive mobile web controller served directly by the daemon:
  - Accessible from any smartphone or tablet on the local LAN without installing an app.
  - Dynamic local IP detection and terminal/app QR code rendering.
  - Real-time bidirectional synchronization with desktop and daemon via WebSockets.

#### CLI & Tooling (`scripts/`)
- `scripts/evo`: Unified command-line interface for Linux:
  - `evo`: Smart launcher (starts daemon if needed and opens desktop app or browser).
  - `evo status`: Live diagnostics of daemon PID, HTTP server, and connected EVO hardware.
  - `evo start` / `evo stop` / `evo restart`: Daemon lifecycle management.
  - `evo qr`: Generates terminal ANSI QR code for instant mobile pairing.
  - `evo dev`: Interactive foreground developer mode with trace logs.
  - `evo app`: Direct desktop application launcher.
- `scripts/install.sh`: Self-contained installer for local user binaries (`~/.local/bin/`), systemd user service, `.desktop` desktop entry, and high-resolution application icons.

#### Automation & Quality
- **GitHub Actions CI** ([`.github/workflows/build.yml`](.github/workflows/build.yml)): Pre-configured build matrix for Rust workspace, daemon packaging, and Tauri desktop binaries (configured for on-demand manual dispatch).
- **Git Hooks & Linters**: Husky pre-commit compilation checks and commitlint Conventional Commits validation.
- **Open Source Guidelines**: Contributor Covenant 2.1 Code of Conduct and detailed Contributing guide.
