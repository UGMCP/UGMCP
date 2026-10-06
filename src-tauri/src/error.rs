use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn auth(message: impl Into<String>) -> Self {
        Self::new("AUTH_FAILED", message)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new("NETWORK_UNAVAILABLE", message)
    }

    pub fn prysel(message: impl Into<String>) -> Self {
        Self::new("SERVER_UNAVAILABLE", message)
    }

    pub fn terminal(message: impl Into<String>) -> Self {
        Self::new("TERMINAL_START_FAILED", message)
    }

    pub fn pty(message: impl Into<String>) -> Self {
        Self::new("PTY_ERROR", message)
    }

    pub fn permission(message: impl Into<String>) -> Self {
        Self::new("PERMISSION_DENIED", message)
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new("STORAGE_ERROR", message)
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new("NOT_CONFIGURED", message)
    }

    pub fn shell(message: impl Into<String>) -> Self {
        Self::new("SHELL_UNAVAILABLE", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("NOT_FOUND", message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

pub fn sanitize_remote_message(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('<') || trimmed.len() > 180 {
        return "Unexpected response from Prysel.".into();
    }
    if trimmed.to_lowercase().contains("secret") || trimmed.to_lowercase().contains("token") {
        return "Authentication failed.".into();
    }
    trimmed.to_string()
}
