use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{config_dir, data_dir};
use crate::error::AppError;
use crate::util::lock;

const KEYRING_SERVICE: &str = "unit-agent";
const KEYRING_USER: &str = "prysel-session";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSettings {
    pub width: f64,
    pub height: f64,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub maximized: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
            x: None,
            y: None,
            maximized: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWorkspace {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub shell: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedMcp {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub debug: bool,
    #[serde(default = "default_font")]
    pub font_size: u16,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub default_cwd: Option<String>,
    #[serde(default = "default_true")]
    pub restore_workspaces: bool,
    #[serde(default)]
    pub workspaces: Vec<SavedWorkspace>,
    #[serde(default)]
    pub active_workspace: Option<String>,
    #[serde(default)]
    pub window: WindowSettings,
    #[serde(default)]
    pub mcp_servers: Vec<SavedMcp>,
}

fn default_theme() -> String {
    "dark".into()
}

fn default_font() -> u16 {
    13
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            debug: false,
            font_size: default_font(),
            shell: None,
            default_cwd: None,
            restore_workspaces: true,
            workspaces: Vec::new(),
            active_workspace: None,
            window: WindowSettings::default(),
            mcp_servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub debug: Option<bool>,
    pub font_size: Option<u16>,
    pub shell: Option<String>,
    pub default_cwd: Option<String>,
    pub restore_workspaces: Option<bool>,
    pub active_workspace: Option<String>,
    pub window: Option<WindowSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSession {
    pub user_id: String,
    pub email: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub nickname: Option<String>,
    pub picture: Option<String>,
    pub role: Option<String>,
    pub scopes: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub auth_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFreshness {
    Valid,
    Expired,
}

impl StoredSession {
    pub fn freshness(&self, now: i64) -> SessionFreshness {
        if now >= self.expires_at {
            SessionFreshness::Expired
        } else {
            SessionFreshness::Valid
        }
    }
}

pub struct Storage {
    settings_path: PathBuf,
    session_path: PathBuf,
    settings: Mutex<Settings>,
}

impl Storage {
    pub fn open() -> Self {
        let settings_path = config_dir().join("settings.json");
        let session_path = data_dir().join("session.bin");
        let _ = fs::create_dir_all(config_dir());
        let _ = fs::create_dir_all(data_dir());
        let settings = read_settings(&settings_path).unwrap_or_default();
        Self {
            settings_path,
            session_path,
            settings: Mutex::new(settings),
        }
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn update_settings(&self, patch: SettingsPatch) -> Result<Settings, AppError> {
        let mut settings = lock(&self.settings);
        if let Some(theme) = patch.theme {
            if theme == "light" || theme == "dark" {
                settings.theme = theme;
            }
        }
        if let Some(debug) = patch.debug {
            settings.debug = debug;
        }
        if let Some(font) = patch.font_size {
            settings.font_size = font.clamp(10, 22);
        }
        if let Some(shell) = patch.shell {
            settings.shell = if shell.trim().is_empty() {
                None
            } else {
                Some(shell)
            };
        }
        if let Some(cwd) = patch.default_cwd {
            settings.default_cwd = if cwd.trim().is_empty() {
                None
            } else {
                Some(cwd)
            };
        }
        if let Some(restore) = patch.restore_workspaces {
            settings.restore_workspaces = restore;
        }
        if let Some(active) = patch.active_workspace {
            settings.active_workspace = if active.is_empty() {
                None
            } else {
                Some(active)
            };
        }
        if let Some(window) = patch.window {
            settings.window = window;
        }
        self.write_settings(&settings)?;
        Ok(settings.clone())
    }

    pub fn save_workspace(&self, workspace: SavedWorkspace) -> Result<(), AppError> {
        let mut settings = lock(&self.settings);
        if let Some(existing) = settings
            .workspaces
            .iter_mut()
            .find(|w| w.id == workspace.id)
        {
            *existing = workspace;
        } else {
            settings.workspaces.push(workspace);
        }
        self.write_settings(&settings)
    }

    pub fn remove_workspace(&self, id: &str) -> Result<(), AppError> {
        let mut settings = lock(&self.settings);
        settings.workspaces.retain(|w| w.id != id);
        if settings.active_workspace.as_deref() == Some(id) {
            settings.active_workspace = None;
        }
        self.write_settings(&settings)
    }

    pub fn update_workspace_cwd(&self, id: &str, cwd: &str) {
        let mut settings = lock(&self.settings);
        if let Some(existing) = settings.workspaces.iter_mut().find(|w| w.id == id) {
            if existing.cwd != cwd {
                existing.cwd = cwd.to_string();
                let _ = self.write_settings(&settings);
            }
        }
    }

    pub fn save_mcp(&self, server: SavedMcp) -> Result<(), AppError> {
        let mut settings = lock(&self.settings);
        if let Some(existing) = settings.mcp_servers.iter_mut().find(|s| s.id == server.id) {
            *existing = server;
        } else {
            settings.mcp_servers.push(server);
        }
        self.write_settings(&settings)
    }

    pub fn remove_mcp(&self, id: &str) -> Result<(), AppError> {
        let mut settings = lock(&self.settings);
        settings.mcp_servers.retain(|s| s.id != id);
        self.write_settings(&settings)
    }

    pub fn mcp_servers(&self) -> Vec<SavedMcp> {
        lock(&self.settings).mcp_servers.clone()
    }

    fn write_settings(&self, settings: &Settings) -> Result<(), AppError> {
        if let Some(parent) = self.settings_path.parent() {
            fs::create_dir_all(parent).map_err(|err| AppError::storage(err.to_string()))?;
        }
        let json = serde_json::to_string_pretty(settings)
            .map_err(|err| AppError::storage(err.to_string()))?;
        atomic_write(&self.settings_path, json.as_bytes())
    }

    pub fn save_session(&self, session: &StoredSession) -> Result<(), AppError> {
        let json =
            serde_json::to_string(session).map_err(|err| AppError::storage(err.to_string()))?;
        if keyring_set(&json).is_ok() {
            let _ = fs::remove_file(&self.session_path);
            return Ok(());
        }
        log::info!("secret service unavailable; storing the session in an encrypted local file");
        let encrypted = encrypt(json.as_bytes()).map_err(AppError::storage)?;
        if let Some(parent) = self.session_path.parent() {
            fs::create_dir_all(parent).map_err(|err| AppError::storage(err.to_string()))?;
        }
        atomic_write(&self.session_path, &encrypted)
    }

    pub fn load_session(&self) -> Result<Option<StoredSession>, AppError> {
        if let Some(json) = keyring_get() {
            return parse_session(&json);
        }
        if !self.session_path.exists() {
            return Ok(None);
        }
        let bytes =
            fs::read(&self.session_path).map_err(|err| AppError::storage(err.to_string()))?;
        let plain = decrypt(&bytes).map_err(AppError::storage)?;
        let json = String::from_utf8(plain)
            .map_err(|_| AppError::storage("Session file is unreadable."))?;
        parse_session(&json)
    }

    pub fn clear_session(&self) -> Result<(), AppError> {
        let _ = keyring_delete();
        if self.session_path.exists() {
            fs::remove_file(&self.session_path)
                .map_err(|err| AppError::storage(err.to_string()))?;
        }
        Ok(())
    }
}

fn parse_session(json: &str) -> Result<Option<StoredSession>, AppError> {
    let session: StoredSession =
        serde_json::from_str(json).map_err(|_| AppError::storage("Stored session is invalid."))?;
    Ok(Some(session))
}

fn read_settings(path: &PathBuf) -> Result<Settings, AppError> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = fs::read_to_string(path).map_err(|err| AppError::storage(err.to_string()))?;
    serde_json::from_str(&text).map_err(|err| AppError::storage(err.to_string()))
}

fn atomic_write(path: &PathBuf, bytes: &[u8]) -> Result<(), AppError> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = open_private(&tmp).map_err(|err| AppError::storage(err.to_string()))?;
        file.write_all(bytes)
            .map_err(|err| AppError::storage(err.to_string()))?;
        file.sync_all().ok();
    }
    fs::rename(&tmp, path).map_err(|err| AppError::storage(err.to_string()))?;
    Ok(())
}

fn open_private(path: &Path) -> Result<File, std::io::Error> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

fn keyring_set(secret: &str) -> Result<(), ()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let secret = secret.to_string();
    std::thread::spawn(move || {
        let result = (|| {
            let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|_| ())?;
            entry.set_password(&secret).map_err(|_| ())
        })();
        let _ = tx.send(result);
    });
    rx.recv_timeout(std::time::Duration::from_millis(1500))
        .unwrap_or(Err(()))
}

fn keyring_get() -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
            entry.get_password().ok()
        })();
        let _ = tx.send(result);
    });
    rx.recv_timeout(std::time::Duration::from_millis(1500))
        .ok()
        .flatten()
}

fn keyring_delete() -> Result<(), ()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|_| ())?;
            entry.delete_credential().map_err(|_| ())
        })();
        let _ = tx.send(result);
    });
    rx.recv_timeout(std::time::Duration::from_millis(1500))
        .unwrap_or(Err(()))
}

fn machine_key() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(machine_identity().trim().as_bytes());
    hasher.update(b"|unit-agent-session-v1|");
    hasher.update(user_identity().as_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

fn machine_identity() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Some(id) =
            read_trimmed("/etc/machine-id").or_else(|| read_trimmed("/var/lib/dbus/machine-id"))
        {
            return id;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(id) = macos_platform_uuid() {
            return id;
        }
    }
    #[cfg(windows)]
    {
        if let Some(id) = windows_machine_guid() {
            return id;
        }
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unit-agent-machine".into())
}

fn read_trimmed(path: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn user_identity() -> String {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid().to_string() }
    }
    #[cfg(windows)]
    {
        std::env::var("USERNAME").unwrap_or_else(|_| "user".into())
    }
    #[cfg(not(any(unix, windows)))]
    {
        "user".into()
    }
}

#[cfg(target_os = "macos")]
fn macos_platform_uuid() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let output = std::process::Command::new("/usr/sbin/ioreg")
                .args(["-rd1", "-c", "IOPlatformExpertDevice"])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let Some(rest) = line.split("IOPlatformUUID").nth(1) else {
                    continue;
                };
                let Some(start) = rest.find('"') else {
                    continue;
                };
                let after = &rest[start + 1..];
                let Some(end) = after.find('"') else {
                    continue;
                };
                let uuid = after[..end].trim().to_string();
                if !uuid.is_empty() {
                    return Some(uuid);
                }
            }
            None
        })
        .clone()
}

#[cfg(windows)]
fn windows_machine_guid() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let output = std::process::Command::new("reg")
                .args([
                    "query",
                    r"HKLM\SOFTWARE\Microsoft\Cryptography",
                    "/v",
                    "MachineGuid",
                ])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if line.contains("MachineGuid") {
                    let guid = line.split_whitespace().last()?.to_string();
                    if !guid.is_empty() && guid != "MachineGuid" {
                        return Some(guid);
                    }
                }
            }
            std::env::var("COMPUTERNAME").ok()
        })
        .clone()
}

fn encrypt(plain: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(&machine_key()).map_err(|_| "cipher".to_string())?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plain)
        .map_err(|_| "encrypt".to_string())?;
    let mut out = nonce_bytes.to_vec();
    out.extend(ciphertext);
    Ok(out)
}

fn decrypt(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 13 {
        return Err("session file is too short".into());
    }
    let cipher = Aes256Gcm::new_from_slice(&machine_key()).map_err(|_| "cipher".to_string())?;
    let nonce = Nonce::from_slice(&bytes[..12]);
    cipher
        .decrypt(nonce, &bytes[12..])
        .map_err(|_| "session file could not be decrypted".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_session_roundtrip() {
        let session = StoredSession {
            user_id: "user-1".into(),
            email: "ada@prysel.com".into(),
            name: Some("Ada".into()),
            username: Some("ada".into()),
            nickname: None,
            picture: Some("https://auth.prysel.com/avatar.png".into()),
            role: Some("user".into()),
            scopes: "profile email".into(),
            issued_at: 10,
            expires_at: 20,
            auth_url: "https://auth.prysel.com".into(),
        };
        let json = serde_json::to_string(&session).unwrap();
        let encrypted = encrypt(json.as_bytes()).unwrap();
        assert_ne!(encrypted, json.as_bytes());
        let plain = decrypt(&encrypted).unwrap();
        let restored: StoredSession = serde_json::from_slice(&plain).unwrap();
        assert_eq!(restored.email, "ada@prysel.com");
        assert_eq!(restored.freshness(15), SessionFreshness::Valid);
        assert_eq!(restored.freshness(20), SessionFreshness::Expired);
    }

    #[test]
    fn settings_patch_clamps_font_and_theme() {
        let dir = std::env::temp_dir().join(format!("unit-agent-test-{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&dir);
        let storage = Storage {
            settings_path: dir.join("settings.json"),
            session_path: dir.join("session.bin"),
            settings: Mutex::new(Settings::default()),
        };
        let updated = storage
            .update_settings(SettingsPatch {
                theme: Some("light".into()),
                debug: Some(true),
                font_size: Some(99),
                shell: Some("  ".into()),
                default_cwd: None,
                restore_workspaces: Some(false),
                active_workspace: None,
                window: None,
            })
            .unwrap();
        assert_eq!(updated.theme, "light");
        assert_eq!(updated.font_size, 22);
        assert!(updated.shell.is_none());
        assert!(!updated.restore_workspaces);
        let _ = fs::remove_dir_all(&dir);
    }
}
