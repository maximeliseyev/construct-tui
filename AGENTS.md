# AGENTS.md — construct-tui

Context for AI agents working in this repository.

---

## What is construct-tui?

Terminal UI client for Construct Messenger. Built with Rust + [Ratatui](https://ratatui.rs).
Runs on macOS, Linux, Raspberry Pi. Binary name: `konstruct`.

**This client is behind iOS and is not currently a production-usable messenger.**
`construct-messenger` is the source of truth for how the protocol actually works.
Do not treat the April 2026 TUI guide as current — it lives in
`construct-docs/_archive/TUI_DEVELOPMENT_GUIDE.md`.

---

## Current stack (read before changing anything)

Two parallel integration paths exist. Neither is finished.

```
screens/  →  app.rs
              ├── orchestrator_task.rs  →  construct-core Orchestrator   ← crypto / sessions
              ├── transport.rs          →  construct-transport QuicClient ← QUIC/H3 (not wired yet)
              └── streaming.rs          →  dummy loop (not a live stream)
```

- Crypto decisions belong in `construct-core::orchestration::Orchestrator`.
  `orchestrator_task.rs` is the platform bridge (storage, timers, UI events).
- `construct-engine` is **gone from this crate**. Same path as iOS/Android:
  `construct-core` + `construct-transport` (+ VEIL later). Do not reintroduce
  the engine.
- `streaming.rs` is a stub. `fetch_bundle_json` and auth RPCs are not wired to
  transport yet.
- `--bridge` / `--headless` / `--config` are clap flags, not behaviour.

Toolchain: stable ≥ 1.96. `Cargo.lock` must be regenerated after the engine drop.

---

## Architecture

```
main.rs
├── app.rs               — App state, event loop
├── orchestrator_task.rs — construct-core Orchestrator actor (preferred crypto path)
├── bridge.rs            — PlatformBridge + UI events from the orchestrator
├── transport.rs         — construct-transport QuicClient (connection not opened yet)
├── proto.rs             — prost types from construct-protos
├── streaming.rs         — intended gRPC stream; currently a dummy loop
├── storage.rs           — SQLCipher messages / sessions / acks
├── auth.rs              — registration / restore (RPCs not wired yet)
├── invite.rs            — invite link handling
├── tui.rs               — terminal setup / teardown
├── event.rs             — TUI input event types
├── config/              — ~/.config/construct-tui/  (session.enc, config.json)
└── screens/             — Ratatui widgets (chats, chat, settings, login, register…)
```

Screens borrow from `App`. They must not call `construct-core` internals or open
their own gRPC. Crypto goes through `orchestrator_task`; network I/O will go
through `transport.rs`.

---

## Build & Run

Sibling path deps: `../construct-core`, `../construct-transport`, `../construct-protos`.
Toolchain: stable ≥ 1.96.

```bash
cargo build --release              # binary at target/release/konstruct
cargo run                          # dev mode
cargo test                         # tests (storage + safety number only)
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Cross-compilation (when the crate actually builds):

```bash
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

Default features include `post-quantum`. There is **no** `ice` feature.

---

## Key conventions

- New protocol work follows iOS/`construct-core`, not the engine.
- TUI screens are in `screens/` — Ratatui `Widget` impls.
- State lives in `app.rs` `App` — screens borrow from it.
- User config: `~/.config/construct-tui/` (not `~/.config/konstruct/`).
- Data: `~/.local/share/construct-tui/` (`messages.db`, `konstruct.log`).

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
