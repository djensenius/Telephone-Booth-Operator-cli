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
- **Theming & settings**: Catppuccin and Bell-Canada colour palettes, optional
  Nerd Font icons, a `?` help overlay, a Settings screen (connection, configured
  booths, identity) and an About screen.

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
tb-operator --setup         # re-run the interactive setup flow
tb-operator --version       # print the version and exit

# Admin-only, non-interactive data backup (see below):
tb-operator data export --output backup.tar   # download a full archive
tb-operator data import --input backup.tar    # restore a full archive
```

### First-run setup

On first launch (when no config file exists yet) `tb-operator` runs an
interactive **setup flow** in the terminal — you can also re-run it any time
with `tb-operator --setup`. Each prompt is preceded by a short explanation of
what it configures. It:

1. prompts for the operator API URL and, optionally, custom Authentik OIDC
   settings (the defaults target the production Telephone-Booth tenant);
2. lets you pick a colour theme and toggle Nerd Font icons;
3. collects any booth debug-server connections (explaining the debug token and
   the pinned TLS certificate fingerprint);
4. signs you in with the device-authorization grant (see
   [Authentication](#authentication)), showing the code and verification URL,
   opening your browser when possible, and offering to retry if it fails; and
5. validates the operator API connection with the new token.

The shareable configuration is written to the config directory, while sensitive
booth debug tokens are written to a separate owner-only secrets file in the
platform data directory (see [Configuration](#configuration)). Setup is skipped
automatically when the input is not an interactive terminal.

### Navigation and keys

| Key | Action |
| --- | --- |
| `?` | Toggle the help overlay (also offers log in / sign out) |
| `Tab` / `→` | Next screen |
| `Shift-Tab` / `←` | Previous screen |
| `1`–`9`, `0`, `s`, `a` | Jump via the screen palette shortcuts |
| `j` / `k` / `↑` / `↓` | Move the selection within a list |
| `r` / `R` | Refresh the active screen |
| `L` | Log in (Authentik device code) |
| `O` | Sign out (Settings screen or help overlay) |
| `q` / `Esc` / `Ctrl-C` | Quit |

Press **`?`** at any time for a grouped screen palette and your current account
status, with log in / sign out actions. The screens are
**Status, Messages, Questions, Events, Sessions, Statistics, Live System, System
Health, Debug, API Tokens, Settings,** and **About**. Each screen's available
actions are shown in the footer hint bar — for example, Messages offers
approve/reject, translate, re-transcribe/re-moderate, delete, and audio playback
(`p` play, `space` pause, `s` stop). You can log in with `L` from anywhere; sign
out with `O` from the Settings screen or the help overlay, and cycle the colour
theme with `t` on Settings. Settings can also edit common config values in-app:
`u` operator API URL, `b` booth debug URL, `k` booth debug token, and `p` poll
interval.

With Nerd Font icons enabled (the default), the header, status bar, toasts, and
account status are decorated with glyphs; they require a
[Nerd Font](https://www.nerdfonts.com/) in your terminal and can be turned off
during setup or with the `nerd-fonts` config key.

The Debian package also installs a `man tb-operator` page and shell completions
for bash, zsh, and fish.

### Operator roles (admin vs. read-only)

Operator accounts come in two tiers, derived live from Authentik group
membership by the operator API and reported on `GET /v1/auth/me`:

- **Administrators** can manage questions (activate, deactivate, archive, and
  create) and run the data export/import commands below.
- **Regular operators** get a read-only Questions screen; the management keys
  (`a`/`e`/`d`/`n`) surface a short "requires an administrator account" hint
  instead of acting.

`tb-operator` re-validates the signed-in identity about once a minute. If the
account has been deleted or removed from the operator group in Authentik, the
next check signs you out automatically rather than trusting the cached token, so
a revoked account cannot keep operating.

### Admin data backup (export / import)

Administrators can take a full backup of an instance — the database plus **all**
audio (content-addressed by SHA-256) — and restore it, without launching the
UI:

```sh
tb-operator data export --output telephone-booth-backup.tar
tb-operator data import --input  telephone-booth-backup.tar
```

Both commands use your stored operator session token (log in once with the TUI
if needed) and call the admin-only `/v1/admin/data` endpoints; a non-admin
session is rejected by the server. `export` writes the raw `.tar` archive to the
given path; `import` uploads it and prints a summary of the rows restored and
how many audio blobs were uploaded versus skipped (already present).

## Authentication

`tb-operator` signs in to the operator API with Authentik's OAuth 2.0 **device
authorization grant**, so no password is ever typed into the terminal:

1. Launch `tb-operator` and press **`L`** (from any screen, or open the `?` help
   overlay and press `L`). The Settings screen and the help overlay show a short
   user code and a verification URL.
2. `tb-operator` opens that URL in your browser when it can; otherwise open it
   on any device.
3. Enter the code and approve the request.
4. Sign-in then completes automatically. The **refresh token is stored in your
   operating system's keychain** (never in the config file); the access token is
   refreshed transparently as it expires.

Press **`O`** on the Settings screen or in the help overlay to sign out and clear
the cached token. The issuer, client id, and scopes can be overridden in the
config file; the defaults target the production Telephone-Booth Authentik tenant.

To configure your own Authentik tenant (or reuse the mobile app's provider for
the console's device-code flow), see [`docs/authentik-setup.md`](./docs/authentik-setup.md).

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
# Themes: catppuccin-mocha (default), catppuccin-macchiato, catppuccin-frappe,
# catppuccin-latte, bell-canada, bell-canada-blue, high-contrast.
theme = "catppuccin-mocha"
nerd-fonts = true            # set false for plain-text labels (no Nerd Font glyphs)
poll-interval-ms = 5000

# Repeat the [[booths]] block for each booth's on-device debug server. For
# Tailscale, use the HTTPS URL printed by `telephone-booth tailscale-status`
# or `scripts/setup-tailscale-serve.sh` on the booth, not `:8080` directly.
[[booths]]
id = "booth-1"
name = "Lobby"               # optional display name
debug-base-url = "https://telephone-booth.example.ts.net/"
debug-token = "…"            # optional static bearer for the debug server
pinned-sha256 = "…"          # optional LAN TLS cert fingerprint (lower-case hex)
```

The Authentik refresh token is **not** stored in this file — it lives in the OS
keychain. The per-booth `debug-token` is sensitive too, so the setup flow keeps
it out of this file: it is written to a separate, owner-only secrets file in the
platform **data** directory (e.g.
`~/.local/share/tb-operator/secrets.toml` on Linux, or
`~/Library/Application Support/io.telephonebooth.tb-operator/secrets.toml` on
macOS) and merged back in at startup. An inline `debug-token` in `config.toml`
is still honoured for backwards compatibility.

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
