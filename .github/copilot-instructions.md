# Copilot instructions — Telephone-Booth-Operator-CLI

This repository is a Rust **terminal UI** operator console (binary
**`tb-operator`**) for the Telephone-Booth installation. It mirrors the web and
mobile operator apps and adds a `btm`-style system-health dashboard.

## Stack & layout

- Rust workspace, edition 2024, toolchain **1.96.0** (`rust-toolchain.toml`).
- UI: `ratatui` + `crossterm`; async: `tokio`; HTTP: `reqwest` (rustls);
  WebSockets: `tokio-tungstenite`; audio: `rodio` + `symphonia`.
- Crates: `tbo-core`, `tbo-auth`, `tbo-operator-client`, `tbo-booth-client`,
  `tbo-metrics`, `tbo-tui` (the binary).

## Conventions

- **Conventional commits.** Default to `fix:` (patch). Use `feat:` only for
  genuinely new functionality and `feat!:` for breaking changes.
- **Do not** add a `Co-authored-by: Copilot` trailer to commits or PRs.
- Workspace lints are strict: `unsafe_code` is forbidden, clippy `pedantic` and
  `nursery` are on, and CI runs clippy with `-D warnings`. Keep code
  warning-clean and avoid `unwrap`/`expect` outside tests.
- Public items need doc comments (`missing_docs` is enabled).
- Format with `cargo fmt`; run `cargo clippy --workspace --all-targets
  --all-features` before pushing.

## Backends

- **Operator API**: Authentik bearer JWT, REST + SSE (`/v1/events/stream`
  accepts a bearer header; `/v1/ws/status` is cookie-only and unused here).
- **Booth debug server**: static debug token (Bearer), REST + telemetry
  WebSocket + Prometheus `/metrics`; LAN TLS uses a pinned self-signed cert.

## PR workflow

CI must pass and all review feedback must be addressed before a PR is
squash-merged. Releases are automated with release-please.
