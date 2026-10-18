# Contributing to EVO Linux Control Suite

Thank you for your interest in contributing to **EVO Linux Control Suite**! This project provides a native, open-source Linux control daemon and desktop interface for Audient EVO audio interfaces (EVO 4, EVO 8, and EVO 16).

---

## 🛠️ Requirements & Development Setup

### System Prerequisites (Ubuntu / Debian)
```bash
sudo apt update
sudo apt install -y build-essential curl wget file pkg-config \
  libasound2-dev libudev-dev libssl-dev libgtk-3-dev \
  libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev
```

*(On Arch Linux / CachyOS / Fedora, install the equivalent `alsa-lib`, `systemd-libs`, `webkit2gtk-4.1`, `libappindicator-gtk3` packages).*

### Tooling Prerequisites
- **Rust >= 1.85** (2024 Edition) — install via [rustup](https://rustup.rs/)
- **Node.js >= 20** & **pnpm >= 10** or **Bun >= 1.2** — for frontend and git hooks

### Repository Setup
```bash
# 1. Clone repository
git clone https://github.com/arsian/evo-linux-control-suite.git
cd evo-linux-control-suite

# 2. Install workspace dependencies and activate Husky Git hooks
pnpm install

# 3. Verify workspace compilation
cargo check --workspace
```

---

## 💻 Development Commands

| Command | Action |
|---|---|
| `cargo check --workspace` | Verify compilation across all crates |
| `cargo test --workspace --exclude evo-desktop` | Run domain, protocol, and service unit tests |
| `cargo run -p evo-daemon` | Run the system daemon locally in development mode |
| `cd apps/desktop && bun run dev` | Run the desktop React frontend with hot reload in browser |
| `cd apps/desktop && bun run tauri dev` | Run the full desktop Tauri v2 application in dev mode |
| `cd apps/desktop && bun run tauri build` | Compile the production release binary of the desktop app |
| `./scripts/evo status` | Inspect local daemon status and hardware diagnostics |

---

## 🪝 Git Hooks & Commit Conventions

This repository enforces **[Conventional Commits](https://www.conventionalcommits.org/)** via **Husky** and **commitlint**:

- **`pre-commit`**: Automatically runs `cargo check` to prevent committing broken code.
- **`commit-msg`**: Validates the commit message format.

### Commit Format
```text
type(scope): subject
```

- **Types**: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
- **Common Scopes**:
  - `daemon`: Changes to the background system daemon (`crates/evo-daemon`).
  - `desktop`: Changes to the Tauri desktop app (`apps/desktop`).
  - `remote-web`: Changes to the mobile web app (`apps/remote-web`).
  - `domain`: Domain models and business rules (`crates/evo-domain`).
  - `core`: Core application services (`crates/evo-core`).
  - `scripts`: CLI and installer scripts (`scripts/evo`, `scripts/install.sh`).
  - `ci`: GitHub Actions workflows (`.github/workflows/`).
  - `docs`: Documentation, architecture, and diagrams.

**Examples**:
```bash
git commit -m "feat(desktop): add direct numeric dB input to central dial"
git commit -m "fix(daemon): debounce USB reconnection polling loop"
git commit -m "docs: update hardware compatibility list"
```

To bypass hooks in exceptional circumstances: `git commit --no-verify -m "..."`.

---

## 🤝 Pull Request Process

1. Fork the repository and create your feature branch from `main`:
   ```bash
   git checkout -b feat/my-new-feature
   ```
2. Ensure your changes compile and pass tests:
   ```bash
   cargo test --workspace --exclude evo-desktop
   ```
3. Commit with Conventional Commits format.
4. Push to your fork and submit a Pull Request describing:
   - What problem is solved or feature is introduced.
   - Any hardware testing conducted (e.g. tested on physical EVO 4 or mock adapter).
5. All pull requests are reviewed before merging.

---

## 📜 Code of Conduct

This project adheres to the **Contributor Covenant 2.1 Code of Conduct**. By participating, you are expected to uphold this code. Please read [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for details.
