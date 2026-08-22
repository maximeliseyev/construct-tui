# construct-tui

Terminal UI client for [Konstruct](https://konstruct.cc) — E2EE messenger with a terminal/ASCII aesthetic.

Built with Rust + [Ratatui](https://ratatui.rs). Target platforms: Linux, macOS, Raspberry Pi. Binary name: `konstruct`.

**Status (2026-08):** early-stage. The crate does not currently build against the live `construct-core` (stale path deps, retired `construct-engine`). The iOS app in `construct-messenger` is the source of truth for protocol behaviour. This repo is being brought back in line.

```
┌─ CONSTRUCT ─────────────────────────────────────────────────────────────────┐
│ > CONTACTS                 │ [alice]  15:42                                 │
│   alice           15:42    │ hey, got the new build running                 │
│   bob             14:11    │                                                │
│   carol           11:03    │ [you]  15:43                                   │
│                            │ works on the Pi Zero too now                   │
│                            │                                                │
│                            │ ▌                                              │
└────────────────────────────┴────────────────────────────────────────────────┘
```

License: [MPL-2.0](LICENSE). Trademark: [TRADEMARK.md](TRADEMARK.md).

---

## Terminal compatibility

| Terminal | Support |
|----------|---------|
| **WezTerm** | Recommended — true color, Unicode, ligatures |
| **Kitty** | Excellent |
| **iTerm2** | Good |
| **Alacritty** | Good (no ligatures) |
| **tmux** | Works — set `TERM=xterm-256color` or `tmux-256color` |
| Apple Terminal | 256 colors only, no true color |

Minimum terminal size: **80×24**.

---

## Requirements

| Dependency | Version |
|------------|---------|
| Rust toolchain | stable ≥ 1.96 (matches `construct-core`) |
| Sibling checkouts | `../construct-core`, `../construct-transport`, `../construct-protos` |
| `libsqlcipher` | bundled (no system install needed) |

```bash
# macOS
brew install rustup
rustup toolchain install 1.96.0

# Debian / Ubuntu
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install 1.96.0
```

Layout expected by `Cargo.toml`:

```
~/Code/
  construct-tui/       # this repo
  construct-core/      # crypto
  construct-engine/    # current path dependency — retired from iOS/Android; see AGENTS.md
```

---

## Build

```bash
cargo build --release
# binary: target/release/konstruct
```

Post-quantum (Kyber-768 PQXDH) is **on by default**. To build without it:

```bash
cargo build --release --no-default-features
```

> **Raspberry Pi Zero W:** Kyber-768 handshake can take ~60 s. Use `--no-default-features` on very small boards.

There is no `ice` / obfs4 feature. `construct-ice` was retired in favour of VEIL; the CLI still accepts `--bridge` but it only fills a config label.

Copy a built binary to another machine:

```bash
scp ./target/release/konstruct pi@raspberrypi.local:/usr/local/bin/konstruct
```

---

## Run

```bash
# Defaults — server from ~/.config/construct-tui/config.json,
# or https://ams.konstruct.cc:443 if the file is missing
konstruct

# Override server
konstruct --server https://ams.konstruct.cc:443

# Disable at-rest encryption (headless / systemd use)
konstruct --no-encrypt
# or:
CONSTRUCT_NO_ENCRYPT=1 konstruct

# Log level: error, warn, info, debug, trace (default: info).
# Logs: ~/.local/share/construct-tui/konstruct.log
# RUST_LOG overrides --log-level when set.
konstruct --log-level debug

# Print log path and exit
konstruct log-path

# Delete local session and all keys
konstruct logout
```

`--bridge` / `--bridge-tls-sni`, `--headless`, and `--config` are parsed by clap but **not wired** — they do not change transport, skip the TUI, or load a custom config file.

---

## First run

1. **Register** — enter a username. The client generates Ed25519 + X25519 keys locally, solves a proof-of-work challenge, and registers the device with the server.
2. **Set passphrase** — protects the session file and message database with Argon2id + AES-256-GCM at rest. Leave empty to skip encryption (or pass `--no-encrypt`).
3. **Chat** — main screen.

On subsequent runs the session is loaded from disk and decrypted with the passphrase.

End-to-end messaging against the live server / iOS client is **not yet working** (message stream is a stub; pre-key bundle fetch is unimplemented).

---

## Key bindings

### Main screen

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate contact list |
| `Enter` | Open conversation |
| `Tab` / `i` | Focus compose box |
| `Esc` | Back to contact list |
| `Shift+Tab` | Focus contact list from compose |
| `Enter` (in compose) | Send message |
| `s` | Open settings |
| `a` | Add contact (search) |
| `q` | Quit (when compose is not focused) |
| `Ctrl+C` | Force quit (any screen) |

### Settings screen

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate |
| `Enter` | Select / confirm |
| `Esc` / `q` | Back to chat |

### Add contact (search overlay)

| Key | Action |
|-----|--------|
| Type | Search by username |
| `↑` / `↓` | Navigate results |
| `Ctrl+A` | Add selected contact |
| `Esc` | Close |

---

## Config file

Stored at `~/.config/construct-tui/config.json`. Created automatically on first run.

```json
{
  "server": "https://ams.konstruct.cc:443",
  "transport": {
    "mode": "Direct"
  }
}
```

Only `Direct` is used. Other `mode` values (`Obfs4`, `Obfs4Tls`, `CdnFront`) are stored and shown in settings; they do not select a transport.

---

## Data storage

| File | Contents |
|------|----------|
| `~/.config/construct-tui/session.enc` | Encrypted session (keys + tokens). Argon2id + AES-256-GCM. |
| `~/.config/construct-tui/config.json` | Server URL + transport config (plaintext). |
| `~/.local/share/construct-tui/messages.db` | SQLCipher-encrypted message database. |
| `~/.local/share/construct-tui/konstruct.log` | Application log. |

Argon2id parameters: 32 MiB memory, 3 iterations, 1 thread (tuned for Raspberry Pi 4). The DB key is derived from the same master key via HKDF.

**Deleting everything:**

```bash
rm -rf ~/.config/construct-tui ~/.local/share/construct-tui
# or:
konstruct logout
```

---

## Development

```bash
cargo run
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
bash scripts/install-hooks.sh    # pre-commit: fmt + clippy
```

There is no CI in this repo (removed 2026-06-19; the project was too early-stage to build in GitHub Actions).

---

## Security notes

- **Keys never leave the device** — the server only stores public keys.
- **Session file** is encrypted with Argon2id + AES-256-GCM. The Argon2id salt is stored alongside the ciphertext.
- **Messages** are stored in a SQLCipher AES-256 encrypted database.
- **Signal Protocol** (X3DH + Double Ratchet) + **PQXDH** (Kyber-768) when built with default features.

DPI-bypass (VEIL) is **not** integrated here. Transport is `construct-transport` (QUIC/H3), same crate iOS uses; the TUI has not opened a live connection yet.

## Trademark

**Konstruct™** / **Конструкт™** and the logo are trademarks of Maxim Eliseyev. The open-source
license on this code does **not** grant trademark rights — see [TRADEMARK.md](TRADEMARK.md).
Forks that distribute a modified version must rebrand.
