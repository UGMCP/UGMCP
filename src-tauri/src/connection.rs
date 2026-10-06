use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use crate::config::PryselConfig;
use crate::util::{lock, now_secs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reach {
    Unknown,
    Checking,
    Online,
    Offline,
    Error,
}

impl Reach {
    pub fn as_str(self) -> &'static str {
        match self {
            Reach::Unknown => "UNKNOWN",
            Reach::Checking => "CHECKING",
            Reach::Online => "ONLINE",
            Reach::Offline => "OFFLINE",
            Reach::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub state: String,
    pub internet: String,
    pub prysel: String,
    pub checked_at: Option<i64>,
    pub detail: String,
}

impl ConnectionSnapshot {
    pub fn checking() -> Self {
        Self {
            state: "CHECKING".into(),
            internet: Reach::Checking.as_str().into(),
            prysel: Reach::Unknown.as_str().into(),
            checked_at: None,
            detail: "Checking connectivity".into(),
        }
    }
}

pub fn derive_state(internet: Reach, prysel: Reach) -> (&'static str, &'static str) {
    match (internet, prysel) {
        (Reach::Checking | Reach::Unknown, _) => ("CHECKING", "Checking connectivity"),
        (Reach::Offline, _) => (
            "OFFLINE",
            "Internet is unavailable. Local terminals keep working.",
        ),
        (Reach::Online, Reach::Online) => ("ONLINE", "Internet and Prysel are reachable."),
        (Reach::Online, Reach::Checking | Reach::Unknown) => ("CHECKING", "Checking Prysel"),
        (Reach::Online, _) => ("DEGRADED", "Internet is available. Prysel is unavailable."),
        (Reach::Error, Reach::Online) => ("DEGRADED", "Prysel is reachable."),
        (Reach::Error, _) => ("ERROR", "Connection check failed."),
    }
}

pub struct ConnectionService {
    http: reqwest::Client,
    snapshot: Mutex<ConnectionSnapshot>,
}

impl ConnectionService {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(2))
            .user_agent(concat!("UnitAgent/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            snapshot: Mutex::new(ConnectionSnapshot::checking()),
        }
    }

    pub fn snapshot(&self) -> ConnectionSnapshot {
        lock(&self.snapshot).clone()
    }

    pub async fn check(&self, config: &PryselConfig) -> ConnectionSnapshot {
        let tcp = tcp_probe();
        let mut prysel = prysel_probe(&self.http, &config.api_url).await;
        let internet = if tcp || prysel == Reach::Online {
            Reach::Online
        } else {
            Reach::Offline
        };
        if internet == Reach::Offline {
            prysel = Reach::Offline;
        }
        let (state, detail) = derive_state(internet, prysel);
        let snapshot = ConnectionSnapshot {
            state: state.into(),
            internet: internet.as_str().into(),
            prysel: prysel.as_str().into(),
            checked_at: Some(now_secs()),
            detail: detail.into(),
        };
        *lock(&self.snapshot) = snapshot.clone();
        snapshot
    }
}

fn tcp_probe() -> bool {
    for addr in ["1.1.1.1:443", "8.8.8.8:443", "9.9.9.9:443"] {
        let Ok(mut addrs) = addr.to_socket_addrs() else {
            continue;
        };
        let Some(socket) = addrs.next() else {
            continue;
        };
        if TcpStream::connect_timeout(&socket, Duration::from_secs(2)).is_ok() {
            return true;
        }
    }
    false
}

async fn prysel_probe(http: &reqwest::Client, base: &str) -> Reach {
    let base = base.trim_end_matches('/');
    match probe_health(http, &format!("{base}/health")).await {
        Reach::Online => Reach::Online,
        other => {
            if matches!(
                probe_config(http, &format!("{base}/api/config")).await,
                Reach::Online
            ) {
                return Reach::Online;
            }
            other
        }
    }
}

async fn probe_health(http: &reqwest::Client, url: &str) -> Reach {
    match http.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(value) if value.get("status").and_then(|s| s.as_str()) == Some("ok") => {
                Reach::Online
            }
            _ => Reach::Error,
        },
        Ok(_) => Reach::Error,
        Err(err) if err.is_timeout() || err.is_connect() || err.is_request() => Reach::Offline,
        Err(_) => Reach::Error,
    }
}

async fn probe_config(http: &reqwest::Client, url: &str) -> Reach {
    match http.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(value) if value.get("ssoEnabled").and_then(|s| s.as_bool()) == Some(true) => {
                Reach::Online
            }
            _ => Reach::Error,
        },
        Ok(_) => Reach::Error,
        Err(err) if err.is_timeout() || err.is_connect() || err.is_request() => Reach::Offline,
        Err(_) => Reach::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_online_offline_and_degraded() {
        assert_eq!(derive_state(Reach::Online, Reach::Online).0, "ONLINE");
        assert_eq!(derive_state(Reach::Offline, Reach::Online).0, "OFFLINE");
        assert_eq!(derive_state(Reach::Online, Reach::Offline).0, "DEGRADED");
        assert_eq!(derive_state(Reach::Online, Reach::Error).0, "DEGRADED");
        assert_eq!(derive_state(Reach::Checking, Reach::Unknown).0, "CHECKING");
        assert_eq!(derive_state(Reach::Error, Reach::Error).0, "ERROR");
    }
}
