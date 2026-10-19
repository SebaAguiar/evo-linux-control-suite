## What

<!-- Brief summary of what this pull request changes or implements. -->

## Why

<!-- Problem solved, background context, linked issues (e.g. Closes #12). -->

## Hardware & Environment Tested

- [ ] Audient EVO 4
- [ ] Audient EVO 8
- [ ] Audient EVO 16
- [ ] Linux Distribution: <!-- e.g. Ubuntu 24.04, Arch Linux, Fedora -->
- [ ] Audio Server: PipeWire / PulseAudio / ALSA

## How to Test

<!-- Concrete commands or steps to verify this change. -->
```bash
cargo check --workspace --exclude evo-desktop
cargo test --workspace --exclude evo-desktop
```

## Checklist

- [ ] Code follows project formatting and conventions.
- [ ] Commits adhere to Conventional Commits specification.
- [ ] Workspace compiles cleanly without warnings or errors.
- [ ] Unit & integration tests pass.
- [ ] Relevant documentation / CHANGELOG updated if applicable.
