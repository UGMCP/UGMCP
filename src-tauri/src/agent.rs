use std::time::Duration;

use serde::Serialize;

use crate::config::PryselConfig;
use crate::system::detect_shell;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub name: String,
    pub available: bool,
    pub detail: String,
}

pub struct LocalAgentService;

impl LocalAgentService {
    pub fn status() -> AgentStatus {
        let shell = detect_shell(None).unwrap_or_else(|_| "/bin/sh".into());
        AgentStatus {
            name: "local".into(),
            available: true,
            detail: format!(
                "Local shell {shell} is available. Local model execution is not configured."
            ),
        }
    }
}

pub struct PryselAgentService {
    http: reqwest::Client,
}

impl PryselAgentService {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { http }
    }

    pub async fn status(&self, config: &PryselConfig) -> AgentStatus {
        let Some(url) = config.agent_url.clone() else {
            return AgentStatus {
                name: "prysel".into(),
                available: false,
                detail: "Online agent service is not configured.".into(),
            };
        };
        let health = format!("{url}/health");
        match self.http.get(&health).send().await {
            Ok(resp) if resp.status().is_success() => AgentStatus {
                name: "prysel".into(),
                available: true,
                detail: "Prysel agent endpoint responded.".into(),
            },
            Ok(_) => AgentStatus {
                name: "prysel".into(),
                available: false,
                detail: "Prysel agent endpoint is unavailable.".into(),
            },
            Err(_) => AgentStatus {
                name: "prysel".into(),
                available: false,
                detail: "Prysel agent endpoint is unreachable.".into(),
            },
        }
    }
}
