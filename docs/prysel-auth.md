# Prysel Auth integration

Authentication provider: `https://auth.prysel.com`

SDK: [CerveauAnalytique/prysel.auth](https://github.com/CerveauAnalytique/prysel.auth.git), package `@prysel/sso` (`packages/prysel-sso`). A copy of that package is in `vendor/prysel-sso`.

The SDK is a small TypeScript client:

| Method | HTTP |
| --- | --- |
| `createAuthorizeUrl` | `POST {authBaseUrl}/api/sso/sessions` |
| `exchangeCode` | `POST {authBaseUrl}/api/sso/token` |
| `parseCallback` | reads `code`, `state`, and `error` from the callback URL |

The session body is:

```json
{
  "app_id": "<PRYSEL_APP_ID>",
  "client_secret": "<PRYSEL_CLIENT_SECRET>",
  "redirect_uri": "http://127.0.0.1:47821/callback",
  "state": "<random>",
  "scopes": "profile email phone"
}
```

The token body is:

```json
{
  "code": "<code>",
  "app_id": "<PRYSEL_APP_ID>",
  "client_secret": "<PRYSEL_CLIENT_SECRET>",
  "redirect_uri": "http://127.0.0.1:47821/callback"
}
```

A successful exchange returns:

```json
{
  "user": {
    "id": "...",
    "email": "...",
    "name": "...",
    "username": "...",
    "nickname": "...",
    "picture": "https://..."
  },
  "scopes": "profile email phone"
}
```

Unit Agent's Rust client in `src-tauri/src/auth.rs` sends those bodies. It does not invent a second login protocol and it does not embed a username/password form. Tests in that module lock the JSON field names to the SDK.

## Desktop callback

`@prysel/sso` redirects the browser to `redirect_uri`. A website uses an HTTPS route. Unit Agent binds `127.0.0.1` and uses:

```text
http://127.0.0.1:47821/callback
```

Prysel validates the redirect by scheme, hostname, and port (`isValidCallback` in the auth server). Register that exact origin on the Unit Agent application record, for example:

```text
http://127.0.0.1:47821/callback
```

The server returns `authorize_url` (typically `https://auth.prysel.com/company/{appId}/{token}`). Unit Agent opens it in the system browser. The user signs in on the Prysel domain and approves the application. Prysel then redirects to the loopback URL with `code` and `state`.

If the user declines, the callback contains `error=access_denied` and Unit Agent shows an authentication error without leaving offline mode unavailable.

## What is stored

The SSO response does not include a refresh token or an Auth0 access token. Unit Agent stores the returned profile and a local expiry (`PRYSEL_SESSION_TTL_HOURS`, default 7 days). There is no silent refresh call to make. When the local expiry passes, the login screen offers **Sign in again** and **Continue Offline**.

Logout deletes the local session. It does not call a remote revoke endpoint because the SDK does not return a token to revoke. The browser SSO cookie, if any, remains in the system browser until that browser session ends.

## Reachability

`GET {PRYSEL_API_URL}/health` is treated as Prysel-online when the JSON body is `{ "status": "ok" }`. If that route is not the API, Unit Agent falls back to `GET /api/config` and looks for `ssoEnabled: true`. A generic internet connection with neither response is `DEGRADED`: local terminals stay available.

## Configuration checklist

1. Create an SSO application in Prysel Auth for Unit Agent.
2. Set its client secret and callback URL as above.
3. Put `PRYSEL_APP_ID` and `PRYSEL_CLIENT_SECRET` in `~/.config/unit-agent/config.env` or the environment.
4. Leave `PRYSEL_AUTH_URL` and `PRYSEL_API_URL` at `https://auth.prysel.com` unless the API is hosted separately. The production auth host proxies `/api` to the auth API.
5. Do not put the client secret in frontend code, CI logs, or the git repository.
