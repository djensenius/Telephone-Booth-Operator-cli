# Telephone-Booth-Operator-CLI

A Rust **terminal UI** (TUI) operator console for the
[Telephone-Booth](https://github.com/djensenius/Telephone-Booth) art
installation. The binary is named **`tb-operator`**.

It brings the web ([Telephone-Booth-Operator]) and mobile
([Telephone-Booth-Mobile]) operator experiences to the terminal, and adds a
[`btm`](https://github.com/ClementTsang/bottom)-style live **system-health
dashboard** sourced from the booth's Prometheus `/metrics` endpoint.

[Telephone-Booth-Operator]: https://github.com/djensenius/Telephone-Booth-Operator
[Telephone-Booth-Mobile]: https://github.com/djensenius/Telephone-Booth-Mobile

## Status

Early development. The workspace is being built up in phases; see the issues and
milestones for the roadmap.

## Features (planned)

- **Operator console** (via the Authentik-secured operator API):
  status, messages (moderation, translation, transcription, playback),
  questions, events (live tail), sessions, stats, API tokens.
- **System health (`btm` style)**: live CPU / load / memory / disk / network /
  temperature / uptime charts scraped from the booth's `/metrics`.
- **Debug panel**: live state, GPIO, audio meters, logs, config, and event
  simulation via the booth's on-device debug server.
- **In-terminal audio**: FLAC playback of recorded messages and questions.
- **Authentik authentication** via the OAuth 2.0 **device authorization grant**,
  with refresh tokens stored in the OS keychain.

## Architecture

The console talks to two backends:

| Backend           | Auth                         | Transport                       |
| ----------------- | ---------------------------- | ------------------------------- |
| **Operator API**  | Authentik bearer JWT         | REST + SSE                      |
| **Booth debug**   | static debug token (Bearer)  | REST + telemetry WS + `/metrics`|

### Workspace crates

| Crate                  | Responsibility                                            |
| ---------------------- | --------------------------------------------------------- |
| `tbo-core`             | Domain types, configuration, errors                       |
| `tbo-auth`             | Authentik device-code flow, token refresh, keychain       |
| `tbo-operator-client`  | Operator REST + bearer SSE client                         |
| `tbo-booth-client`     | Booth debug HTTP + telemetry WS + TLS pinning             |
| `tbo-metrics`          | Prometheus text parser, ring buffers, rate calculation    |
| `tbo-tui`              | The `tb-operator` binary: ratatui UI, input, audio        |

## Building

Requires Rust **1.95.0** (pinned via `rust-toolchain.toml`).

On Linux the audio backend needs ALSA development headers:

```sh
sudo apt-get install -y libasound2-dev pkg-config
```

Then:

```sh
cargo build
cargo run -p tbo-tui   # runs the `tb-operator` binary
```

## Installation

### Homebrew (macOS, Apple Silicon)

```sh
brew tap djensenius/tap
brew install telephone-booth-operator-cli
```

### Debian / Ubuntu (amd64, arm64, armhf)

Releases ship as `.deb` packages through the shared Telephone-Booth APT
repository. Add the repo once (this is the same keyring and source line the
booth package uses), then install:

```sh
curl -fsSL https://djensenius.github.io/Telephone-Booth/telephone-booth-archive-keyring.gpg \
  | sudo install -m 0644 /dev/stdin /usr/share/keyrings/telephone-booth-archive-keyring.gpg

echo "deb [signed-by=/usr/share/keyrings/telephone-booth-archive-keyring.gpg] https://djensenius.github.io/Telephone-Booth stable main" \
  | sudo tee /etc/apt/sources.list.d/telephone-booth.list

sudo apt update
sudo apt install telephone-booth-operator-cli
```

The binary is installed as `/usr/bin/tb-operator`.

## License

[Apache-2.0](./LICENSE)
