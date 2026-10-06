use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct PryselConfig {
    pub auth_url: String,
    pub api_url: String,
    pub app_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub callback_port: u16,
    pub session_ttl_secs: i64,
    pub agent_url: Option<String>,
}

impl PryselConfig {
    pub fn load() -> Self {
        let mut env_map = std::env::vars().collect::<std::collections::HashMap<_, _>>();
        for path in config_env_paths() {
            if let Ok(text) = fs::read_to_string(&path) {
                for (key, value) in parse_env(&text) {
                    env_map.entry(key).or_insert(value);
                }
            }
        }

        let auth_url = env_map
            .get("PRYSEL_AUTH_URL")
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://auth.prysel.com".into());
        let api_url = env_map
            .get("PRYSEL_API_URL")
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| auth_url.clone());
        let callback_port = env_map
            .get("PRYSEL_CALLBACK_PORT")
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p != 0)
            .unwrap_or(47821);
        let redirect_uri = env_map
            .get("PRYSEL_REDIRECT_URI")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("http://127.0.0.1:{callback_port}/callback"));
        let ttl_hours = env_map
            .get("PRYSEL_SESSION_TTL_HOURS")
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|h| *h > 0)
            .unwrap_or(168);
        let agent_url = env_map
            .get("PRYSEL_AGENT_URL")
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty());

        Self {
            auth_url,
            api_url,
            app_id: env_map.get("PRYSEL_APP_ID").cloned().unwrap_or_default(),
            client_secret: env_map
                .get("PRYSEL_CLIENT_SECRET")
                .cloned()
                .unwrap_or_default(),
            redirect_uri,
            callback_port,
            session_ttl_secs: ttl_hours.saturating_mul(3600),
            agent_url,
        }
    }

    pub fn configured(&self) -> bool {
        !self.app_id.trim().is_empty() && !self.client_secret.trim().is_empty()
    }

    pub fn require_configured(&self) -> Result<(), AppError> {
        if self.configured() {
            Ok(())
        } else {
            Err(AppError::config(
                "Prysel Auth is not configured. Set PRYSEL_APP_ID and PRYSEL_CLIENT_SECRET. You can still work offline.",
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicConfig {
    pub auth_url: String,
    pub api_url: String,
    pub app_id: String,
    pub secret_configured: bool,
    pub redirect_uri: String,
    pub callback_port: u16,
    pub agent_configured: bool,
}

impl From<&PryselConfig> for PublicConfig {
    fn from(config: &PryselConfig) -> Self {
        Self {
            auth_url: config.auth_url.clone(),
            api_url: config.api_url.clone(),
            app_id: config.app_id.clone(),
            secret_configured: !config.client_secret.is_empty(),
            redirect_uri: config.redirect_uri.clone(),
            callback_port: config.callback_port,
            agent_configured: config.agent_url.is_some(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("unit-agent")
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("unit-agent")
}

fn config_env_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from(".env"),
        config_dir().join("config.env"),
        config_dir().join(".env"),
    ]
}

pub fn parse_env(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut value = value.trim().to_string();
        if value.len() >= 2 {
            let bytes = value.as_bytes();
            if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
            {
                value = value[1..value.len() - 1].to_string();
            }
        }
        pairs.push((key.to_string(), value));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_without_keeping_quotes() {
        let pairs = parse_env(
            "# comment\nPRYSEL_APP_ID=unit-agent\nPRYSEL_AUTH_URL=\"https://auth.prysel.com\"\n",
        );
        assert_eq!(pairs[0], ("PRYSEL_APP_ID".into(), "unit-agent".into()));
        assert_eq!(pairs[1].1, "https://auth.prysel.com");
    }
}
