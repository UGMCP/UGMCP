use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::PryselConfig;
use crate::error::{sanitize_remote_message, AppError};
use crate::storage::{SessionFreshness, Storage, StoredSession};
use crate::util::{lock, now_secs};

/// Native desktop adapter for the `@prysel/sso` SDK.
///
/// The TypeScript SDK (`vendor/prysel-sso`) posts the client secret from a
/// server route. Unit Agent performs the same HTTP calls from Rust so the
/// secret never enters the webview.
pub struct AuthService {
    http: reqwest::Client,
    config: PryselConfig,
    storage: Arc<Storage>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    online_this_process: Mutex<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicUser {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub nickname: Option<String>,
    pub picture: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub authenticated: bool,
    pub expired: bool,
    pub source: String,
    pub user: Option<PublicUser>,
    pub issued_at: Option<i64>,
    pub expires_at: Option<i64>,
}

impl SessionView {
    pub fn none() -> Self {
        Self {
            authenticated: false,
            expired: false,
            source: "none".into(),
            user: None,
            issued_at: None,
            expires_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackParts {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug)]
enum CallbackResult {
    Ok { code: String },
    Error(String),
    Cancelled,
    Timeout,
}

impl AuthService {
    pub fn new(config: PryselConfig, storage: Arc<Storage>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(2))
            .user_agent(concat!("UnitAgent/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            config,
            storage,
            cancel: Mutex::new(None),
            online_this_process: Mutex::new(false),
        }
    }

    pub fn restore(&self) -> Result<SessionView, AppError> {
        let Some(session) = self.storage.load_session()? else {
            return Ok(SessionView::none());
        };
        match session.freshness(now_secs()) {
            SessionFreshness::Expired => Ok(SessionView {
                authenticated: false,
                expired: true,
                source: "local".into(),
                user: Some(public_user(&session)),
                issued_at: Some(session.issued_at),
                expires_at: Some(session.expires_at),
            }),
            SessionFreshness::Valid => Ok(SessionView {
                authenticated: true,
                expired: false,
                source: if *lock(&self.online_this_process) {
                    "online".into()
                } else {
                    "local".into()
                },
                user: Some(public_user(&session)),
                issued_at: Some(session.issued_at),
                expires_at: Some(session.expires_at),
            }),
        }
    }

    pub async fn login(&self) -> Result<SessionView, AppError> {
        self.config.require_configured()?;
        if self.cancel.lock().ok().and_then(|g| g.clone()).is_some() {
            return Err(AppError::auth("Authentication is already in progress."));
        }
        let state = random_token(24);
        let port = self.config.callback_port;
        let redirect = redirect_for_port(&self.config.redirect_uri, port)?;
        let (rx, stop) = start_callback_server(port, state.clone())?;
        *lock(&self.cancel) = Some(stop.clone());

        let login_result = self.login_inner(&redirect, &state, rx).await;
        stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(200),
        );
        *lock(&self.cancel) = None;
        login_result
    }

    async fn login_inner(
        &self,
        redirect: &str,
        state: &str,
        rx: mpsc::Receiver<CallbackResult>,
    ) -> Result<SessionView, AppError> {
        let body = authorize_body(
            &self.config.app_id,
            &self.config.client_secret,
            redirect,
            Some(state),
        );
        let url = format!(
            "{}/api/sso/sessions",
            self.config.api_url.trim_end_matches('/')
        );
        let response = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest)?;
        if !response.status().is_success() {
            return Err(error_from_status(response).await);
        }
        let payload: Value = response.json().await.map_err(|_| {
            AppError::new("UNEXPECTED_RESPONSE", "Unexpected authentication response.")
        })?;
        let authorize_url = payload
            .get("authorize_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::new(
                    "UNEXPECTED_RESPONSE",
                    "Prysel did not return an authorization URL.",
                )
            })?;
        if !authorize_url.starts_with("https://")
            && !authorize_url.starts_with("http://127.0.0.1")
            && !authorize_url.starts_with("http://localhost")
        {
            return Err(AppError::new(
                "UNEXPECTED_RESPONSE",
                "Prysel returned an authorization URL that is not allowed.",
            ));
        }
        log::info!("opening the system browser for Prysel Auth");
        open::that(authorize_url)
            .map_err(|_| AppError::auth("Could not open the system browser for Prysel Auth."))?;

        let callback = tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_secs(310))
                .unwrap_or(CallbackResult::Timeout)
        })
        .await
        .map_err(|_| AppError::auth("Authentication failed."))?;

        let code = match callback {
            CallbackResult::Ok { code } => code,
            CallbackResult::Cancelled => {
                return Err(AppError::new("AUTH_CANCELLED", "Authentication cancelled."));
            }
            CallbackResult::Timeout => {
                return Err(AppError::new("AUTH_FAILED", "Authentication timed out."));
            }
            CallbackResult::Error(message) => return Err(AppError::auth(message)),
        };

        let exchange = exchange_body(
            &code,
            &self.config.app_id,
            &self.config.client_secret,
            redirect,
        );
        let token_url = format!(
            "{}/api/sso/token",
            self.config.api_url.trim_end_matches('/')
        );
        let response = self
            .http
            .post(&token_url)
            .json(&exchange)
            .send()
            .await
            .map_err(map_reqwest)?;
        if !response.status().is_success() {
            return Err(error_from_status(response).await);
        }
        let payload: Value = response.json().await.map_err(|_| {
            AppError::new("UNEXPECTED_RESPONSE", "Unexpected authentication response.")
        })?;
        let user = payload.get("user").cloned().unwrap_or(Value::Null);
        let scopes = payload
            .get("scopes")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let session = stored_from_user(&user, &scopes, &self.config)?;
        self.storage.save_session(&session)?;
        *lock(&self.online_this_process) = true;
        log::info!("prysel session established for user {}", session.user_id);
        self.restore()
    }

    pub fn cancel(&self) {
        if let Some(flag) = lock(&self.cancel).clone() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn logout(&self) -> Result<SessionView, AppError> {
        self.cancel();
        self.storage.clear_session()?;
        *lock(&self.online_this_process) = false;
        log::info!("local prysel session cleared");
        Ok(SessionView::none())
    }
}

pub fn authorize_body(
    app_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    state: Option<&str>,
) -> Value {
    json!({
        "app_id": app_id,
        "client_secret": client_secret,
        "redirect_uri": redirect_uri,
        "state": state,
        "scopes": "profile email phone"
    })
}

pub fn exchange_body(code: &str, app_id: &str, client_secret: &str, redirect_uri: &str) -> Value {
    json!({
        "code": code,
        "app_id": app_id,
        "client_secret": client_secret,
        "redirect_uri": redirect_uri
    })
}

/// Same fields as `PryselSSO.parseCallback` in `@prysel/sso`.
pub fn parse_callback(url: &str) -> Result<CallbackParts, AppError> {
    let parsed =
        url::Url::parse(url).map_err(|_| AppError::new("INVALID_CALLBACK", "Invalid callback."))?;
    Ok(CallbackParts {
        code: parsed
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned()),
        state: parsed
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned()),
        error: parsed
            .query_pairs()
            .find(|(k, _)| k == "error")
            .map(|(_, v)| v.into_owned()),
    })
}

fn stored_from_user(
    user: &Value,
    scopes: &str,
    config: &PryselConfig,
) -> Result<StoredSession, AppError> {
    let id = user.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        AppError::new(
            "UNEXPECTED_RESPONSE",
            "Prysel did not return a user profile.",
        )
    })?;
    let email = user
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let now = now_secs();
    Ok(StoredSession {
        user_id: id.to_string(),
        email,
        name: optional_string(user, "name"),
        username: optional_string(user, "username"),
        nickname: optional_string(user, "nickname"),
        picture: safe_picture(optional_string(user, "picture")),
        role: optional_string(user, "role"),
        scopes: scopes.to_string(),
        issued_at: now,
        expires_at: now.saturating_add(config.session_ttl_secs),
        auth_url: config.auth_url.clone(),
    })
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn safe_picture(url: Option<String>) -> Option<String> {
    url.filter(|value| value.starts_with("https://") && value.len() < 2048)
}

fn public_user(session: &StoredSession) -> PublicUser {
    PublicUser {
        id: session.user_id.clone(),
        email: session.email.clone(),
        name: session.name.clone(),
        username: session.username.clone(),
        nickname: session.nickname.clone(),
        picture: session.picture.clone(),
        role: session.role.clone(),
    }
}

fn redirect_for_port(configured: &str, port: u16) -> Result<String, AppError> {
    let parsed = url::Url::parse(configured).map_err(|_| {
        AppError::new(
            "INVALID_CALLBACK",
            "PRYSEL_REDIRECT_URI is not a valid URL.",
        )
    })?;
    if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") {
        return Err(AppError::new(
            "INVALID_CALLBACK",
            "The desktop callback must be http://127.0.0.1:<port>/callback.",
        ));
    }
    let parsed_port = parsed.port().unwrap_or(80);
    if parsed_port != port {
        return Err(AppError::new(
            "INVALID_CALLBACK",
            "PRYSEL_REDIRECT_URI port must match PRYSEL_CALLBACK_PORT.",
        ));
    }
    Ok(configured.to_string())
}

fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn start_callback_server(
    port: u16,
    expected_state: String,
) -> Result<(mpsc::Receiver<CallbackResult>, Arc<AtomicBool>), AppError> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|_| {
        AppError::new(
            "AUTH_FAILED",
            format!("Could not listen on 127.0.0.1:{port} for the Prysel callback. Close the program using that port or set PRYSEL_CALLBACK_PORT."),
        )
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|err| AppError::auth(err.to_string()))?;
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline && !stop_thread.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let result = interpret_request(&request, &expected_state);
                    let page = match &result {
                        CallbackResult::Ok { .. } => SUCCESS_PAGE,
                        CallbackResult::Cancelled => CANCEL_PAGE,
                        _ => ERROR_PAGE,
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                        page.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = tx.send(result);
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(_) => break,
            }
        }
        if stop_thread.load(Ordering::SeqCst) {
            let _ = tx.send(CallbackResult::Cancelled);
        } else {
            let _ = tx.send(CallbackResult::Timeout);
        }
    });
    Ok((rx, stop))
}

fn interpret_request(request: &str, expected_state: &str) -> CallbackResult {
    let first = request.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "GET" || !target.starts_with("/callback") {
        return CallbackResult::Error("Invalid callback.".into());
    }
    let url = format!("http://127.0.0.1{target}");
    let Ok(parsed) = parse_callback(&url) else {
        return CallbackResult::Error("Invalid callback.".into());
    };
    if let Some(error) = parsed.error {
        let message = match error.as_str() {
            "access_denied" => "Authentication was declined.",
            _ => "Authentication failed.",
        };
        return CallbackResult::Error(message.into());
    }
    if parsed.state.as_deref() != Some(expected_state) {
        return CallbackResult::Error("Invalid callback.".into());
    }
    match parsed.code {
        Some(code) if !code.is_empty() && code.len() < 512 => CallbackResult::Ok { code },
        _ => CallbackResult::Error("Invalid callback.".into()),
    }
}

fn map_reqwest(err: reqwest::Error) -> AppError {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        AppError::network("Network unavailable.")
    } else {
        AppError::prysel("Prysel server is unavailable.")
    }
}

async fn error_from_status(response: reqwest::Response) -> AppError {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .or_else(|| value.get("error").and_then(|v| v.as_str()))
                .map(str::to_string)
        })
        .unwrap_or_default();
    let message = sanitize_remote_message(&message);
    if status.as_u16() == 410
        || message.contains("CODE_EXPIRED")
        || message.contains("SESSION_EXPIRED")
    {
        return AppError::new("SESSION_EXPIRED", "Session expired.");
    }
    if message.contains("INVALID_REDIRECT")
        || message.contains("Invalid redirect")
        || message.contains("REDIRECT_MISMATCH")
    {
        return AppError::new(
            "INVALID_CALLBACK",
            "The redirect URI is not registered for this Prysel application. Allow http://127.0.0.1:47821/callback.",
        );
    }
    if status.as_u16() == 401 || message.contains("INVALID_CLIENT") {
        return AppError::auth("Prysel rejected the application credentials.");
    }
    if status.is_server_error() || status.as_u16() == 404 {
        return AppError::prysel("Prysel server is unavailable.");
    }
    if message == "Unexpected response from Prysel." {
        return AppError::new("UNEXPECTED_RESPONSE", message);
    }
    AppError::auth(message)
}

const SUCCESS_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Unit Agent</title></head><body style=\"margin:0;background:#0c1018;color:#f2f4f8;font-family:sans-serif;display:grid;place-items:center;height:100vh\"><p>Signed in. You can return to Unit Agent.</p></body></html>";
const ERROR_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Unit Agent</title></head><body style=\"margin:0;background:#0c1018;color:#f2f4f8;font-family:sans-serif;display:grid;place-items:center;height:100vh\"><p>Authentication did not complete. Return to Unit Agent and try again.</p></body></html>";
const CANCEL_PAGE: &str = ERROR_PAGE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_body_matches_sso_sdk() {
        let body = authorize_body(
            "unit-agent",
            "secret",
            "http://127.0.0.1:47821/callback",
            Some("abc"),
        );
        assert_eq!(body["app_id"], "unit-agent");
        assert_eq!(body["client_secret"], "secret");
        assert_eq!(body["redirect_uri"], "http://127.0.0.1:47821/callback");
        assert_eq!(body["state"], "abc");
        assert_eq!(body["scopes"], "profile email phone");
    }

    #[test]
    fn exchange_body_matches_sso_sdk() {
        let body = exchange_body(
            "code",
            "unit-agent",
            "secret",
            "http://127.0.0.1:47821/callback",
        );
        assert_eq!(body["code"], "code");
        assert_eq!(body["app_id"], "unit-agent");
        assert!(body.get("client_secret").is_some());
        assert_eq!(body["redirect_uri"], "http://127.0.0.1:47821/callback");
    }

    #[test]
    fn parse_callback_matches_sdk() {
        let parts = parse_callback("http://127.0.0.1:47821/callback?code=abc&state=xyz").unwrap();
        assert_eq!(parts.code.as_deref(), Some("abc"));
        assert_eq!(parts.state.as_deref(), Some("xyz"));
        assert!(parts.error.is_none());
        let denied =
            parse_callback("http://127.0.0.1:47821/callback?error=access_denied&state=xyz")
                .unwrap();
        assert_eq!(denied.error.as_deref(), Some("access_denied"));
    }

    #[test]
    fn rejects_state_mismatch_and_reflects_no_secrets() {
        let request = "GET /callback?code=abc&state=wrong HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert!(matches!(
            interpret_request(request, "expected"),
            CallbackResult::Error(_)
        ));
        let ok = "GET /callback?code=abc&state=expected HTTP/1.1\r\n\r\n";
        assert!(matches!(
            interpret_request(ok, "expected"),
            CallbackResult::Ok { .. }
        ));
    }

    #[test]
    fn picture_must_be_https() {
        assert!(safe_picture(Some("https://cdn.example/a.png".into())).is_some());
        assert!(safe_picture(Some("http://cdn.example/a.png".into())).is_none());
        assert!(safe_picture(Some("javascript:alert(1)".into())).is_none());
    }
}
