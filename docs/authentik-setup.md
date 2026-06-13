# Authentik setup

This guide configures [Authentik](https://goauthentik.io/) so the
`tb-operator` terminal console can sign in. The console authenticates to the
**operator API** with an Authentik-issued bearer JWT; this page covers the
Authentik side and the matching `tb-operator` configuration.

It is the terminal counterpart to the other operator clients. If you have
already followed either of these, most of the work is done — see
[Reuse an existing provider](#option-a--reuse-an-existing-provider-recommended):

- Web operator —
  [`Telephone-Booth-Operator/docs/authentik-setup.md`](https://github.com/djensenius/Telephone-Booth-Operator/blob/main/docs/authentik-setup.md)
  (a **confidential** client, authorization-code flow, browser cookie session).
- Mobile / TV apps —
  [`Telephone-Booth-Mobile/docs/auth.md`](https://github.com/djensenius/Telephone-Booth-Mobile/blob/main/docs/auth.md)
  (a **public** client; phones use PKCE redirect, tvOS uses the device-code
  flow that `tb-operator` also relies on).

## How `tb-operator` authenticates

`tb-operator` is a headless terminal app, so it uses the OAuth 2.0 **Device
Authorization Grant** (RFC 8628) — there is no browser or redirect URI on the
machine running the console:

1. The console POSTs `client_id` + `scope` to Authentik's device endpoint and
   shows you a short `user_code` and a verification URL.
2. You approve the request in any browser.
3. The console polls the token endpoint with
   `grant_type=urn:ietf:params:oauth:grant-type:device_code` until Authentik
   returns an `access_token`, `refresh_token`, and `id_token`.
4. The access token is sent as `Authorization: Bearer …` on every operator API
   call and refreshed proactively (~60 s before expiry). The **refresh token is
   stored in your OS keychain**, never in the config file.

Key facts that drive the setup below:

| Property      | Value                                                          |
| ------------- | -------------------------------------------------------------- |
| Client type   | **Public** (no client secret)                                  |
| Grant         | Device authorization grant (RFC 8628) + refresh token          |
| Scopes        | `openid email profile offline_access`                          |
| PKCE          | Not used (device flow); a `PKCE: Required` toggle is harmless  |
| Redirect URI  | None — device flow needs no redirect                           |

`tb-operator` does **not** perform OIDC discovery. It derives the endpoints
from the issuer's parent path (see
[`crates/tbo-auth/src/endpoints.rs`](../crates/tbo-auth/src/endpoints.rs)), so
an issuer of `https://AUTHENTIK_HOST/application/o/<slug>` yields:

- device: `https://AUTHENTIK_HOST/application/o/device/`
- token: `https://AUTHENTIK_HOST/application/o/token/`
- authorize: `https://AUTHENTIK_HOST/application/o/authorize/`

These are Authentik's global OAuth endpoints, so the configured `issuer` **must**
be the full `…/application/o/<slug>` application path — a bare host such as
`https://AUTHENTIK_HOST` will not work.

## Prerequisite: a device-code flow on the brand

This is the one piece the web/PKCE setups do not need, and the most common
reason device login fails. In Authentik the device-code flow is a **brand-level**
setting and **there is no default flow** — you must create one once per tenant.
It then enables device login for every public provider under that brand.

> If tvOS or another device-code client already signs in on your tenant, this
> is already done — skip to
> [provider setup](#option-a--reuse-an-existing-provider-recommended).

Per the
[Authentik device-code docs](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/device_code):

1. **Flows and Stages → Flows → New Flow**:
   - Name / Title: `Device code flow`
   - Slug: `default-device-code-flow`
   - Designation: **Stage Configuration**
   - Authentication: **Require authentication**
2. **System → Brands → edit the default brand**, then set the brand's
   device-code flow (the field the Authentik docs call **Default code flow**)
   to the flow you just created and **Update**.

## Option A — Reuse an existing provider (recommended)

If you already registered the mobile/TV public client
(`telephone-booth-operator-mobile`) per the
[mobile auth doc](https://github.com/djensenius/Telephone-Booth-Mobile/blob/main/docs/auth.md),
`tb-operator` reuses it as-is. It is the console's default issuer and client id,
and the operator API already trusts that provider, so **no API change is
needed**. You only need the brand device-code flow above.

Confirm the provider (Applications → Providers) has:

| Field                       | Value                                            |
| --------------------------- | ------------------------------------------------ |
| Client type                 | **Public**                                       |
| Scopes                      | `openid email profile offline_access`            |
| `groups` claim              | emitted (Authentik's default `profile` mapping)  |
| Device/code validity        | long enough to sign in comfortably (e.g. 5 min)  |
| Refresh token validity      | long enough for your session (e.g. 30 days)      |

…and that the **application** has the `telephone-booth-operators` group bound
under its policy / group bindings, with your user in that group
(Directory → Groups).

Because refresh-token rotation is per-login, the console keeps its own
independent refresh-token lineage in your keychain; sharing a client id with a
phone is fine. Prefer a dedicated client id for a cleaner audit trail? Use
Option B.

The console defaults already point at this provider, so a stock config works.
To pin it explicitly, write
`~/Library/Application Support/io.telephonebooth.tb-operator/config.toml`
(macOS) or `~/.config/tb-operator/config.toml` (Linux):

```toml
[auth]
issuer = "https://auth.fluxhaus.io/application/o/telephone-booth-operator-mobile"
client-id = "x0M0MleMvCSCx8MqIE2jVoYe57nAhGymIG8azTEY"
scopes = "openid email profile offline_access"
```

## Option B — Dedicated CLI provider (a new setup)

Create a separate public provider when you want a distinct client id and audit
trail, different token lifetimes, or you run a different Authentik tenant than
the mobile app. The brand device-code flow above is still required.

### 1. Create the provider

> Applications → Providers → Create → OAuth2/OpenID Provider

| Field                       | Value                                                |
| --------------------------- | ---------------------------------------------------- |
| Name                        | `telephone-booth-operator-cli`                       |
| Authorization flow          | `default-authorization-flow (Authorize Application)` |
| Client type                 | **Public** (no client secret)                        |
| Client ID                   | _auto-generated; copy it_                            |
| Redirect URIs               | _none needed for device flow_                        |
| Signing Key                 | _default (RSA)_                                      |
| Subject mode                | Based on the User's hashed ID                        |
| Include claims in id_token  | Yes                                                  |
| Scopes                      | `openid` `email` `profile` `offline_access`          |
| Device/code validity        | e.g. `minutes=5`                                     |
| Refresh token validity      | e.g. `days=30`                                       |

Make sure the provider emits a `groups` claim (the default `profile` scope
mapping does). If you have removed it, add a scope mapping on `profile`:

```python
# Customization → Property Mappings → Create → Scope Mapping
# Scope name: profile
return {"groups": [group.name for group in user.groups.all()]}
```

### 2. Create the application

> Applications → Applications → Create

| Field    | Value                          |
| -------- | ------------------------------ |
| Name     | `Telephone Booth Operator CLI` |
| Slug     | `telephone-booth-operator-cli` |
| Provider | `telephone-booth-operator-cli` |

Bind the `telephone-booth-operators` group under the application's policy /
group bindings so Authentik refuses non-operators at the authorize step.

### 3. Trust the new provider in the operator API

The operator API only accepts bearer tokens whose `iss`/`aud` it knows. The
primary `OIDC_ISSUER` / `OIDC_CLIENT_ID` are always accepted; **add** the CLI
provider to the mobile/native allow-lists (comma-separated, so multiple native
clients can coexist) and roll the API:

```ini
# Trust the CLI provider's issuer + client id (audience), in addition to the
# primary provider. AUTHENTIK_MOBILE_* are accepted as aliases.
OIDC_MOBILE_ISSUERS=https://auth.fluxhaus.io/application/o/telephone-booth-operator-cli/
OIDC_MOBILE_AUDIENCES=<cli-client-id>

# At least one authorization allow-list is required in production.
OIDC_ALLOWED_GROUPS=telephone-booth-operators
```

If you reused the mobile provider (Option A) and it is already listed here,
there is nothing to change.

### 4. Point the console at it

```toml
[auth]
issuer = "https://auth.fluxhaus.io/application/o/telephone-booth-operator-cli"
client-id = "<cli-client-id>"
scopes = "openid email profile offline_access"
```

## How the operator API validates the bearer

For reference, the API's bearer middleware
([`packages/api/src/lib/bearer-auth.ts`](https://github.com/djensenius/Telephone-Booth-Operator/blob/main/packages/api/src/lib/bearer-auth.ts))
requires the token to:

- be signed by a key from the provider's `jwks_uri`;
- carry `iss` equal to `OIDC_ISSUER` **or** an entry in `OIDC_MOBILE_ISSUERS`;
- carry `aud` equal to `OIDC_CLIENT_ID` **or** an entry in
  `OIDC_MOBILE_AUDIENCES`;
- be unexpired (30 s clock-skew tolerance);
- have a `groups` claim intersecting `OIDC_ALLOWED_GROUPS` (or an email in
  `OIDC_ALLOWED_EMAILS`).

## Sign in

```sh
cargo run -p tbo-tui        # or the installed `tb-operator`
```

Open the **Settings** screen, press **`L`**, then enter the displayed code at
the verification URL on any device. Press **`O`** to sign out (clears the
keychain entry, service `io.telephonebooth.tb-operator`, account
`oidc-session`).

## Verify and troubleshoot

Confirm the tenant advertises the device flow (run on a host that can reach
Authentik):

```sh
curl -s https://auth.fluxhaus.io/application/o/telephone-booth-operator-mobile/.well-known/openid-configuration \
  | python3 -m json.tool \
  | grep -E 'issuer|device_authorization_endpoint|token_endpoint|device_code'
```

You should see a `device_authorization_endpoint` and
`urn:ietf:params:oauth:grant-type:device_code` in `grant_types_supported`.

| Symptom                                   | Likely cause                                                                       |
| ----------------------------------------- | ---------------------------------------------------------------------------------- |
| Device step fails / 400 on `/device/`     | The brand has no device-code flow (see the prerequisite section)                   |
| Sign-in completes but every API call 401s | Operator API does not trust this provider — set `OIDC_MOBILE_ISSUERS`/`_AUDIENCES` |
| API calls return 403                      | Your user is not in `telephone-booth-operators` (or the `groups` claim is missing) |
| Endpoints look wrong / 404                | `issuer` is not the full `…/application/o/<slug>` application path                  |
| `expired_token` before you can approve    | Approve faster, or raise the provider's device/code validity                       |
| Random `iat` / clock-skew errors          | Host clock drift — run `chrony` or `systemd-timesyncd`                              |
