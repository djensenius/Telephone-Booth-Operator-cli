//! First-run onboarding: an interactive, pre-TUI setup flow.
//!
//! Run before the `ratatui` interface takes over the terminal (on first launch
//! when no config file exists, or on demand with `--setup`), this module walks
//! the operator through configuring the console, explaining each prompt as it
//! goes:
//!
//! 1. collect the operator API URL and, optionally, custom Authentik OIDC
//!    settings (defaulting to the production Telephone-Booth tenant);
//! 2. choose the colour theme and whether to render Nerd Font icons;
//! 3. collect any booth debug-server connections;
//! 4. sign in interactively with the OAuth 2.0 device-authorization grant —
//!    showing the verification URL and code, opening the browser when possible,
//!    and offering to retry until it succeeds or is skipped;
//! 5. validate the operator API connection with the freshly issued token.
//!
//! The shareable [`Config`] is written to the platform *config* directory while
//! sensitive booth debug tokens are written to a separate owner-only
//! [`Secrets`] file in the *data* directory (the Authentik refresh token stays
//! in the OS keychain). See [`crate::data`] for the runtime data layer.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::Result;
use time::OffsetDateTime;

use tbo_auth::{InMemoryTokenStore, KeyringTokenStore, SessionManager, TokenStore};
use tbo_core::Secrets;
use tbo_core::config::{AuthConfig, BoothConfig, Config, OperatorConfig, UiConfig};
use tbo_operator_client::OperatorClient;

use crate::data::{SessionTokenProvider, SharedSession};
use crate::ui::theme;

/// The answers collected for a single booth debug-server connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoothAnswer {
    /// Stable booth identifier (matches the API's `boothId`).
    pub id: String,
    /// Optional human-friendly display name.
    pub name: Option<String>,
    /// Debug server base URL.
    pub debug_base_url: String,
    /// Optional static bearer token (stored as a secret, not in the config).
    pub debug_token: Option<String>,
    /// Optional pinned TLS certificate SHA-256 (LAN TLS only).
    pub pinned_sha256: Option<String>,
}

/// The full set of answers gathered during onboarding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingAnswers {
    /// Operator API base URL.
    pub operator_base_url: String,
    /// Authentik OIDC settings for the device-code flow.
    pub auth: AuthConfig,
    /// UI theme name.
    pub theme: String,
    /// Whether to render Nerd Font glyphs in the interface.
    pub nerd_fonts: bool,
    /// Poll interval in milliseconds.
    pub poll_interval_ms: u64,
    /// Configured booths.
    pub booths: Vec<BoothAnswer>,
}

/// Turn the collected answers into a shareable [`Config`] plus the [`Secrets`]
/// (booth debug tokens) split out of it.
#[must_use]
pub fn assemble(answers: OnboardingAnswers) -> (Config, Secrets) {
    let booths = answers
        .booths
        .into_iter()
        .map(|booth| BoothConfig {
            id: booth.id,
            name: booth.name,
            debug_base_url: booth.debug_base_url,
            debug_token: booth.debug_token,
            pinned_sha256: booth.pinned_sha256,
        })
        .collect();
    let mut config = Config {
        operator: OperatorConfig {
            base_url: answers.operator_base_url,
        },
        auth: answers.auth,
        booths,
        ui: UiConfig {
            theme: answers.theme,
            poll_interval_ms: answers.poll_interval_ms,
            nerd_fonts: answers.nerd_fonts,
        },
    };
    let secrets = config.take_secrets();
    (config, secrets)
}

/// Run the interactive onboarding flow, persisting the resulting config and
/// secrets and signing in to the operator API.
pub async fn run(config_path: &Path) -> Result<()> {
    // `Stdout` is `Send`, so it can cross `.await` points; the `StdinLock`
    // guard is not, so it is confined to the synchronous prompt phases below.
    let mut output = io::stdout();

    let mut existing = Config::load_from(config_path)?;
    if let Ok(secrets) = Secrets::load() {
        existing.merge_secrets(&secrets);
    }

    writeln!(output, "tb-operator setup")?;
    writeln!(output, "=================")?;
    writeln!(
        output,
        "Answer a few questions to configure the console. Press Enter to accept"
    )?;
    writeln!(output, "the [default] shown in brackets.\n")?;

    let answers = {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        collect_answers(&mut input, &mut output, &existing)?
    };
    let (config, secrets) = assemble(answers);

    config.save_to(config_path)?;
    secrets.save()?;
    writeln!(output, "\nSaved configuration to {}", config_path.display())?;
    if let Ok(secrets_path) = Secrets::default_path() {
        writeln!(output, "Saved secrets to {}", secrets_path.display())?;
    }

    let session = build_session(&config.auth, &mut output)?;
    if should_sign_in(&session, &mut output)? {
        sign_in_with_retry(&session, &mut output).await?;
    }
    validate(&config, Arc::clone(&session), &mut output).await?;

    writeln!(output, "\nSetup complete. Launching tb-operator…\n")?;
    Ok(())
}

/// Gather all onboarding answers interactively, seeding defaults from any
/// existing configuration so a re-run preserves prior choices. Each section is
/// preceded by a short explanation of what the prompts mean.
fn collect_answers(
    input: &mut impl BufRead,
    output: &mut impl Write,
    existing: &Config,
) -> Result<OnboardingAnswers> {
    section(output, "Operator API")?;
    explain(
        output,
        &[
            "The backend that serves messages, questions, sessions, and stats.",
            "Use the default unless you run your own Telephone-Booth operator API.",
        ],
    )?;
    let operator_base_url = prompt_line(
        input,
        output,
        "Operator API URL",
        &existing.operator.base_url,
    )?;

    section(output, "Authentication (Authentik / OIDC)")?;
    explain(
        output,
        &[
            "Sign-in uses Authentik's OAuth device-code flow (like a smart TV):",
            "no password is typed here — you approve a code in a browser.",
            "The defaults target the production Telephone-Booth tenant.",
        ],
    )?;
    let auth = if prompt_yes_no(
        input,
        output,
        "Use the default Telephone-Booth Authentik (OIDC) settings?",
        existing.auth == AuthConfig::default(),
    )? {
        AuthConfig::default()
    } else {
        explain(
            output,
            &[
                "issuer:    your Authentik application URL, .../application/o/<slug>.",
                "client id: the public client id of that application (no secret).",
                "scopes:    space-separated OAuth scopes (keep offline_access for refresh).",
            ],
        )?;
        AuthConfig {
            issuer: prompt_line(input, output, "OIDC issuer URL", &existing.auth.issuer)?,
            client_id: prompt_line(input, output, "OIDC client id", &existing.auth.client_id)?,
            scopes: prompt_line(input, output, "OIDC scopes", &existing.auth.scopes)?,
        }
    };

    section(output, "Interface")?;
    let theme = prompt_theme(input, output, &existing.ui.theme)?;
    explain(
        output,
        &[
            "Nerd Font icons add glyphs to the tabs and status bar.",
            "They need a Nerd Font (https://nerdfonts.com) installed in your",
            "terminal; choose no to keep plain text labels.",
        ],
    )?;
    let nerd_fonts = prompt_yes_no(
        input,
        output,
        "Enable Nerd Font icons?",
        existing.ui.nerd_fonts,
    )?;

    let booths = collect_booths(input, output, existing)?;

    Ok(OnboardingAnswers {
        operator_base_url,
        auth,
        theme,
        nerd_fonts,
        poll_interval_ms: existing.ui.poll_interval_ms,
        booths,
    })
}

/// Collect booth debug-server connections, optionally keeping any already
/// configured booths and then adding new ones.
fn collect_booths(
    input: &mut impl BufRead,
    output: &mut impl Write,
    existing: &Config,
) -> Result<Vec<BoothAnswer>> {
    section(output, "Booths (optional)")?;
    explain(
        output,
        &[
            "A booth's on-device debug server powers the System Health and Debug",
            "screens. This is only for booths you operate — skip it otherwise; the",
            "messages/questions/stats screens work without any booth configured.",
        ],
    )?;
    let mut booths = Vec::new();
    if !existing.booths.is_empty()
        && prompt_yes_no(
            input,
            output,
            &format!("Keep the {} existing booth(s)?", existing.booths.len()),
            true,
        )?
    {
        booths.extend(existing.booths.iter().map(|booth| BoothAnswer {
            id: booth.id.clone(),
            name: booth.name.clone(),
            debug_base_url: booth.debug_base_url.clone(),
            debug_token: booth.debug_token.clone(),
            pinned_sha256: booth.pinned_sha256.clone(),
        }));
    }

    while prompt_yes_no(input, output, "Add a booth debug connection?", false)? {
        explain(
            output,
            &[
                "id:           the booth's stable identifier (matches the API's boothId).",
                "display name: an optional friendly label shown in the UI.",
                "debug URL:    the debug server, e.g. https://<booth>.<tailnet>.ts.net/",
                "              from Tailscale Serve or https://<lan-ip>:8443 (LAN TLS).",
                "bearer token: the static token the debug server requires; stored in a",
                "              separate owner-only secrets file, never in config.toml.",
                "TLS SHA-256:  for an https:// LAN server with a self-signed certificate,",
                "              paste the cert's SHA-256 fingerprint (lower-case hex) to pin",
                "              it. Leave blank for http:// connections.",
            ],
        )?;
        let Some(id) = prompt_required(input, output, "Booth id")? else {
            break;
        };
        booths.push(BoothAnswer {
            id,
            name: prompt_optional(input, output, "Display name")?,
            debug_base_url: prompt_line(
                input,
                output,
                "Debug server URL",
                "https://telephone-booth.example.ts.net/",
            )?,
            debug_token: prompt_optional(input, output, "Debug bearer token")?,
            pinned_sha256: prompt_optional(input, output, "Pinned TLS SHA-256")?,
        });
    }

    Ok(booths)
}

/// Build a session manager, preferring the OS keychain so the issued refresh
/// token persists for later launches and falling back to an in-memory store
/// (with a warning) when secure storage is unavailable.
fn build_session(auth: &AuthConfig, output: &mut impl Write) -> Result<SharedSession> {
    match keyring_session(auth) {
        Ok(session) => return Ok(session),
        Err(err) => writeln!(
            output,
            "Warning: secure storage unavailable ({err}); the sign-in will not be remembered."
        )?,
    }
    let store: Box<dyn TokenStore> = Box::new(InMemoryTokenStore::new());
    let manager = SessionManager::new(auth, store)?;
    Ok(Arc::new(manager))
}

/// Build a keychain-backed session manager, probing the store so a read failure
/// surfaces here and engages the in-memory fallback.
fn keyring_session(auth: &AuthConfig) -> Result<SharedSession> {
    let store: Box<dyn TokenStore> = Box::new(KeyringTokenStore::new()?);
    let manager = SessionManager::new(auth, store)?;
    manager.current_session()?;
    Ok(Arc::new(manager))
}

/// Decide whether to run the device-authorization sign-in: always when signed
/// out, otherwise only if the operator opts to re-authenticate. The `StdinLock`
/// is confined to this synchronous helper so the async sign-in stays `Send`.
fn should_sign_in(session: &SharedSession, output: &mut impl Write) -> Result<bool> {
    if session.current_session()?.is_none() {
        return Ok(true);
    }
    let stdin = io::stdin();
    let mut input = stdin.lock();
    prompt_yes_no(
        &mut input,
        output,
        "Already signed in. Re-authenticate?",
        false,
    )
}

/// Sign in interactively, retrying on failure until it succeeds or the operator
/// chooses to skip. Prints a short explanation of the device-code flow first.
async fn sign_in_with_retry(session: &SharedSession, output: &mut impl Write) -> Result<()> {
    section(output, "Sign in")?;
    explain(
        output,
        &[
            "We'll show a short code and a URL. Open the URL on any device, enter",
            "the code, and approve the request — then this finishes automatically.",
            "Your refresh token is saved to the OS keychain, never to a file.",
        ],
    )?;
    loop {
        match sign_in_once(session, output).await {
            Ok(()) => {
                writeln!(output, "Signed in.")?;
                return Ok(());
            }
            Err(err) => {
                writeln!(output, "\nSign-in did not complete: {err}")?;
                if !ask_retry(output)? {
                    writeln!(
                        output,
                        "Skipping sign-in. You can log in later from the app: press ? then L,"
                    )?;
                    writeln!(output, "or open Settings and press L.")?;
                    return Ok(());
                }
            }
        }
    }
}

/// One device-authorization attempt: request a code, display it, open the
/// browser when possible, and poll until the operator approves.
async fn sign_in_once(session: &SharedSession, output: &mut impl Write) -> Result<()> {
    writeln!(output, "\nRequesting a device code…")?;
    output.flush()?;
    let authorization = session.client().begin_device_authorization().await?;

    let target = authorization
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&authorization.verification_uri);

    writeln!(output, "\n  To sign in:")?;
    writeln!(
        output,
        "    1. Open:       {}",
        authorization.verification_uri
    )?;
    writeln!(output, "    2. Enter code: {}", authorization.user_code)?;
    writeln!(
        output,
        "    Code expires in: {}",
        format_device_code_lifetime(authorization.expires_in),
    )?;
    if let Some(complete) = &authorization.verification_uri_complete {
        writeln!(output, "    (or open this one-tap link: {complete})")?;
    }
    if try_open_browser(target) {
        writeln!(output, "\n  Opened your browser to the verification page.")?;
    }
    writeln!(output, "\nWaiting for you to approve…")?;
    output.flush()?;

    let tokens = session.client().poll_for_token(&authorization).await?;
    session.complete_login(&tokens, OffsetDateTime::now_utc())?;
    Ok(())
}

/// Ask whether to retry a failed sign-in. The `StdinLock` is confined to this
/// synchronous helper so the async sign-in flow stays `Send`.
fn ask_retry(output: &mut impl Write) -> Result<bool> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    prompt_yes_no(&mut input, output, "Try signing in again?", true)
}

/// Best-effort: open `url` in the platform browser, returning whether the
/// opener launched. Never blocks (the child is not awaited) and never fails the
/// flow — the URL is always printed too.
fn try_open_browser(url: &str) -> bool {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Format the provider-reported device-code lifetime for setup output.
fn format_device_code_lifetime(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        return format!("{seconds}s");
    }
    format!("{}m {:02}s", seconds / 60, seconds % 60)
}

/// Validate the operator API connection by fetching the signed-in profile.
/// A failure is reported but does not abort onboarding.
async fn validate(config: &Config, session: SharedSession, output: &mut impl Write) -> Result<()> {
    writeln!(output, "\nValidating the operator API connection…")?;
    let provider = SessionTokenProvider::new(session);
    let client = match OperatorClient::new(config.operator.base_url.clone(), provider) {
        Ok(client) => client,
        Err(err) => {
            writeln!(output, "Could not build the operator client: {err}")?;
            return Ok(());
        }
    };
    match client.operator_me().await {
        Ok(me) => writeln!(output, "Connected to the operator API as {}.", me.email)?,
        Err(err) => writeln!(
            output,
            "Could not reach the operator API ({err}); you can retry from Settings."
        )?,
    }
    Ok(())
}

/// Print a section header to visually group related prompts.
fn section(output: &mut impl Write, title: &str) -> Result<()> {
    writeln!(output, "\n{title}")?;
    writeln!(output, "{}", "-".repeat(title.len()))?;
    Ok(())
}

/// Print indented explanatory lines beneath a section header.
fn explain(output: &mut impl Write, lines: &[&str]) -> Result<()> {
    for line in lines {
        writeln!(output, "  {line}")?;
    }
    Ok(())
}

/// Prompt for a colour theme, listing the available palettes and accepting
/// either a name or its list number. Returns `default` on empty input.
fn prompt_theme(
    input: &mut impl BufRead,
    output: &mut impl Write,
    default: &str,
) -> Result<String> {
    writeln!(output, "Available colour themes:")?;
    for (index, name) in theme::NAMES.iter().enumerate() {
        writeln!(output, "  {}. {name}", index + 1)?;
    }
    loop {
        let answer = prompt_line(input, output, "Theme (name or number)", default)?;
        if let Ok(number) = answer.parse::<usize>() {
            if let Some(name) = number
                .checked_sub(1)
                .and_then(|index| theme::NAMES.get(index))
            {
                return Ok((*name).to_owned());
            }
        } else if theme::NAMES.contains(&answer.as_str()) {
            return Ok(answer);
        }
        writeln!(output, "Please enter a listed theme name or number.")?;
    }
}

/// Read one trimmed line, returning `None` at end-of-input.
fn read_line(input: &mut impl BufRead) -> Result<Option<String>> {
    let mut buffer = String::new();
    if input.read_line(&mut buffer)? == 0 {
        return Ok(None);
    }
    Ok(Some(buffer.trim().to_owned()))
}

/// Prompt for a value, returning `default` when the input is empty.
fn prompt_line(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    default: &str,
) -> Result<String> {
    write!(output, "{label} [{default}]: ")?;
    output.flush()?;
    match read_line(input)? {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Ok(default.to_owned()),
    }
}

/// Prompt for an optional value, returning `None` when left blank.
fn prompt_optional(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
) -> Result<Option<String>> {
    write!(output, "{label} (optional): ")?;
    output.flush()?;
    match read_line(input)? {
        Some(value) if !value.is_empty() => Ok(Some(value)),
        _ => Ok(None),
    }
}

/// Prompt repeatedly until a non-empty value is given, returning `None` at
/// end-of-input so callers can stop gracefully.
fn prompt_required(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
) -> Result<Option<String>> {
    loop {
        write!(output, "{label}: ")?;
        output.flush()?;
        match read_line(input)? {
            Some(value) if !value.is_empty() => return Ok(Some(value)),
            Some(_) => writeln!(output, "A value is required.")?,
            None => return Ok(None),
        }
    }
}

/// Prompt for a yes/no answer, returning `default_yes` on an empty answer or at
/// end-of-input.
fn prompt_yes_no(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    default_yes: bool,
) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        write!(output, "{label} {hint}: ")?;
        output.flush()?;
        match read_line(input)? {
            None => return Ok(default_yes),
            Some(value) => match value.to_ascii_lowercase().as_str() {
                "" => return Ok(default_yes),
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => writeln!(output, "Please answer y or n.")?,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::Cursor;

    use super::*;

    fn collect(script: &str, existing: &Config) -> OnboardingAnswers {
        let mut input = Cursor::new(script.to_owned());
        let mut output = Vec::new();
        collect_answers(&mut input, &mut output, existing).unwrap()
    }

    #[test]
    fn accepts_defaults_on_empty_input() {
        // Operator URL, default OIDC (yes), theme, nerd fonts, then no booths —
        // all blank to accept each default.
        let answers = collect("\n\n\n\n\n", &Config::default());
        assert_eq!(answers, default_answers());
    }

    fn default_answers() -> OnboardingAnswers {
        let config = Config::default();
        OnboardingAnswers {
            operator_base_url: config.operator.base_url,
            auth: config.auth,
            theme: config.ui.theme,
            nerd_fonts: config.ui.nerd_fonts,
            poll_interval_ms: config.ui.poll_interval_ms,
            booths: Vec::new(),
        }
    }

    #[test]
    fn collects_custom_oidc_and_booth() {
        let script = concat!(
            "https://api.example.test\n", // operator URL
            "n\n",                        // do not use default OIDC
            "https://auth.example/o/app\n",
            "client-xyz\n",
            "openid offline_access\n",
            "4\n",            // theme → catppuccin-latte (by number)
            "n\n",            // disable nerd fonts
            "y\n",            // add a booth
            "booth-1\n",      // id
            "Lobby\n",        // display name
            "\n",             // debug URL → default
            "super-secret\n", // debug token
            "\n",             // pinned sha → none
            "n\n",            // add another booth? no
        );
        let answers = collect(script, &Config::default());

        assert_eq!(answers.operator_base_url, "https://api.example.test");
        assert_eq!(answers.auth.issuer, "https://auth.example/o/app");
        assert_eq!(answers.auth.client_id, "client-xyz");
        assert_eq!(answers.auth.scopes, "openid offline_access");
        assert_eq!(answers.theme, "catppuccin-latte");
        assert!(!answers.nerd_fonts);
        assert_eq!(answers.booths.len(), 1);
        let booth = &answers.booths[0];
        assert_eq!(booth.id, "booth-1");
        assert_eq!(booth.name.as_deref(), Some("Lobby"));
        assert_eq!(
            booth.debug_base_url,
            "https://telephone-booth.example.ts.net/"
        );
        assert_eq!(booth.debug_token.as_deref(), Some("super-secret"));
        assert!(booth.pinned_sha256.is_none());
    }

    #[test]
    fn assemble_moves_booth_token_into_secrets() {
        let answers = OnboardingAnswers {
            operator_base_url: "https://api.example.test".to_owned(),
            auth: AuthConfig::default(),
            theme: "bell-canada".to_owned(),
            nerd_fonts: true,
            poll_interval_ms: 5_000,
            booths: vec![BoothAnswer {
                id: "booth-1".to_owned(),
                name: None,
                debug_base_url: "https://telephone-booth.example.ts.net/".to_owned(),
                debug_token: Some("secret".to_owned()),
                pinned_sha256: None,
            }],
        };

        let (config, secrets) = assemble(answers);

        assert_eq!(config.operator.base_url, "https://api.example.test");
        assert!(config.booths[0].debug_token.is_none());
        assert_eq!(secrets.booth_token("booth-1"), Some("secret"));
        // The serialized config must not leak the secret.
        assert!(!config.to_toml().unwrap().contains("secret"));
    }

    #[test]
    fn prompt_yes_no_parses_answers() {
        let mut output = Vec::new();
        let mut yes = Cursor::new("yes\n".to_owned());
        assert!(prompt_yes_no(&mut yes, &mut output, "?", false).unwrap());
        let mut no = Cursor::new("n\n".to_owned());
        assert!(!prompt_yes_no(&mut no, &mut output, "?", true).unwrap());
        let mut empty = Cursor::new("\n".to_owned());
        assert!(prompt_yes_no(&mut empty, &mut output, "?", true).unwrap());
        let mut eof = Cursor::new(String::new());
        assert!(!prompt_yes_no(&mut eof, &mut output, "?", false).unwrap());
    }

    #[test]
    fn keeps_existing_booths_when_requested() {
        let existing = Config {
            booths: vec![BoothConfig {
                id: "booth-1".to_owned(),
                name: Some("Lobby".to_owned()),
                debug_base_url: "http://localhost:8080".to_owned(),
                debug_token: Some("tok".to_owned()),
                pinned_sha256: None,
            }],
            ..Config::default()
        };
        // operator default, default OIDC, theme, nerd fonts, keep booths (yes),
        // add another (no).
        let answers = collect("\n\n\n\ny\nn\n", &existing);
        assert_eq!(answers.booths.len(), 1);
        assert_eq!(answers.booths[0].debug_token.as_deref(), Some("tok"));
    }
}
