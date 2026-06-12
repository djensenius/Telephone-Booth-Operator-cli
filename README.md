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

Feature-complete across the planned phases: Authentik authentication, the full
operator console (status, messages, questions, events, sessions, statistics,
live system), the booth-direct **System Health** and **Debug** panels,
in-terminal audio playback, and theming with Settings/About screens. See the
issues and milestones for any remaining polish.

## Features

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
- **Theming & settings**: switchable colour palettes, a Settings screen
  (connection, configured booths, identity) and an About screen.

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

## Usage

```sh
tb-operator                 # launch the console with the default config
tb-operator --config FILE   # use an alternate TOML config file
tb-operator --version       # print the version and exit
```

### Navigation and keys

| Key | Action |
| --- | --- |
| `Tab` / `→` | Next screen |
| `Shift-Tab` / `←` | Previous screen |
| `1`–`9` | Jump to a screen by tab number |
| `j` / `k` / `↑` / `↓` | Move the selection within a list |
| `r` / `R` | Refresh the active screen |
| `q` / `Esc` / `Ctrl-C` | Quit |

The screens are **Status, Messages, Questions, Events, Sessions, Statistics,
Live System, System Health, Debug, API Tokens, Settings,** and **About**. Each
screen's available actions are shown in the footer hint bar — for example,
Messages offers approve/reject, translate, re-transcribe/re-moderate, delete,
and audio playback (`p` play, `space` pause, `s` stop); Settings offers `L` log
in, `O` sign out, and `t` to cycle the colour theme.

The Debian package also installs a `man tb-operator` page and shell completions
for bash, zsh, and fish.

## Authentication

`tb-operator` signs in to the operator API with Authentik's OAuth 2.0 **device
authorization grant**, so no password is ever typed into the terminal:

1. Launch `tb-operator` and open the **Settings** screen.
2. Press **`L`** to start signing in. The screen shows a short user code and a
   verification URL.
3. On any device, open that URL, enter the code, and approve the request.
4. Sign-in then completes automatically. The **refresh token is stored in your
   operating system's keychain** (never in the config file); the access token is
   refreshed transparently as it expires.

Press **`O`** on the Settings screen to sign out and clear the cached token. The
issuer, client id, and scopes can be overridden in the config file; the defaults
target the production Telephone-Booth Authentik tenant.

## Configuration

Configuration is read from a TOML file under the platform config directory
(e.g. `~/.config/tb-operator/config.toml` on Linux, or
`~/Library/Application Support/io.telephonebooth.tb-operator/config.toml` on
macOS), or from the path given with `--config`. Every field has a default, so a
fresh install works out of the box against the production operator API; add
booths to enable the **System Health** and **Debug** screens.

```toml
[operator]
base-url = "https://api.telephonebooth.io"

[auth]
issuer = "https://auth.fluxhaus.io/application/o/telephone-booth-operator-mobile"
client-id = "x0M0MleMvCSCx8MqIE2jVoYe57nAhGymIG8azTEY"
scopes = "openid email profile offline_access"

[ui]
theme = "bell-canada"        # or "bell-canada-blue", "high-contrast"
poll-interval-ms = 5000

# Repeat the [[booths]] block for each booth's on-device debug server.
[[booths]]
id = "booth-1"
name = "Lobby"               # optional display name
debug-base-url = "http://localhost:8080"
debug-token = "…"            # optional static bearer for the debug server
pinned-sha256 = "…"          # optional LAN TLS cert fingerprint (lower-case hex)
```

The Authentik refresh token is **not** stored in this file — it lives in the OS
keychain. The per-booth `debug-token` is operator-controlled and is kept here,
mirroring the web Debug screen.

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
