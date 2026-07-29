# AGENTS.md — construct-tui

Context for AI agents working in this repository.

---

## What is construct-tui?

Terminal UI client for Construct Messenger. Built with Rust + [Ratatui](https://ratatui.rs).
Runs on macOS, Linux, Raspberry Pi. Binary name: `konstrukt`.

Uses `construct-engine` (not `construct-core` directly) for all crypto and server comms.

---

## Architecture

```
main.rs
├── app.rs              — App state, event loop
├── engine_adapter.rs   — ConstructEngine wrapper (UiEvent dispatch, PlatformAction handler)
├── bridge.rs           — Bridge between TUI events and engine events
├── streaming.rs        — gRPC message stream handling
├── storage.rs          — Local message/session persistence
├── auth.rs             — Auth flow (registration, login)
├── invite.rs           — Invite link handling
├── orchestrator_task.rs — Background engine task
├── tui.rs              — Terminal setup / teardown
├── event.rs            — TUI input event types
└── screens/            — Screen views (chats, chat, settings, login, register…)
```

### engine_adapter.rs is the integration boundary

All interactions with the Construct protocol go through `EngineAdapter`.
It wraps `ConstructEngine` and translates TUI app events into `UiEvent`s,
and `PlatformAction`s back into TUI state updates.

---

## Build & Run

```bash
cargo build --release              # build — binary at target/release/konstrukt
cargo run                          # run in dev mode
cargo test                         # tests
cargo clippy                       # lint

# Install locally
cargo install --path .
```

Cross-compilation (for release):
```bash
# Linux x86_64
cargo build --release --target x86_64-unknown-linux-gnu
# Linux aarch64 (Raspberry Pi)
cargo build --release --target aarch64-unknown-linux-gnu
```

---

## Key conventions

- All crypto/server operations go through `engine_adapter.rs` — never call `construct-engine` internals directly from screens
- TUI screens are in `screens/` — each screen is a Ratatui `Widget` impl
- State lives in `app.rs` `App` struct — screens borrow from it
- `config/` — user config directory (`~/.config/konstrukt/`)

---
---

## Documentation & session notes

All docs live in `~/Code/construct-docs` (Obsidian vault, flat domain folders:
`architecture/ backend/ client/ cryptocore/ security/ decisions/ sessions/ …`).
**The vault's `AGENTS.md` is authoritative** for structure and writing rules — read it before
contributing docs. If a path is missing, search the domain folder rather than trusting old links.

After any session with architectural changes, design decisions, root-cause analysis, or
non-obvious choices:

1. Write a session note `sessions/YYYY-MM-DD-<topic>.md` (sections: Context / What Changed /
   **Why** / Decisions / Open Questions) — `## Why` with rejected alternatives is mandatory.
2. If it constrains future work, add/update `decisions/<slug>.md`.
3. Patch the affected spec in its domain folder in the **same** session.
4. Append one line to `~/Code/construct-docs/log.md`: `[YYYY-MM-DD HH:MM] note | <topic>`.

Session notes are plain markdown, no YAML frontmatter; `[[wikilinks]]` to other notes are welcome.
Before creating a note, search for an existing one and extend it rather than duplicating.
