use std::path::Path;

use crate::error::AppError;

pub fn home_dir() -> String {
    dirs::home_dir()
        .map(|path| path.to_string_lossy().to_string())
        .or_else(|| std::env::var("USERPROFILE").ok())
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "C:\\".into()
            } else {
                "/".into()
            }
        })
}

pub fn detect_shell(override_shell: Option<&str>) -> Result<String, AppError> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(shell) = override_shell.map(str::trim).filter(|s| !s.is_empty()) {
        candidates.push(shell.to_string());
    }
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.trim().is_empty() {
            candidates.push(shell);
        }
    }
    if let Ok(shell) = std::env::var("COMSPEC") {
        if !shell.trim().is_empty() {
            candidates.push(shell);
        }
    }
    candidates.extend(default_shells());
    for candidate in candidates {
        if Path::new(&candidate).is_file() {
            return Ok(candidate);
        }
    }
    Err(AppError::shell("No usable shell was found on this system."))
}

fn default_shells() -> Vec<String> {
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        vec![
            format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            format!("{root}\\System32\\cmd.exe"),
        ]
    }
    #[cfg(target_os = "macos")]
    {
        vec!["/bin/zsh".into(), "/bin/bash".into(), "/bin/sh".into()]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![
            "/bin/bash".into(),
            "/usr/bin/bash".into(),
            "/bin/zsh".into(),
            "/bin/sh".into(),
        ]
    }
    #[cfg(not(any(windows, unix)))]
    {
        Vec::new()
    }
}

pub fn shell_kind(shell: &str) -> &'static str {
    let lowered = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let name = lowered.strip_suffix(".exe").unwrap_or(&lowered);
    match name {
        "zsh" => "zsh",
        "fish" => "fish",
        "bash" => "bash",
        "powershell" | "pwsh" => "powershell",
        "cmd" => "cmd",
        _ => "other",
    }
}

pub fn has_child_processes(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/task/{pid}/children");
        std::fs::read_to_string(path)
            .map(|text| text.split_whitespace().any(|part| !part.is_empty()))
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        command_has_output("/usr/bin/pgrep", &["-P", &pid.to_string()])
    }
    #[cfg(windows)]
    {
        let script = format!(
            "(Get-CimInstance Win32_Process -Filter \"ParentProcessId={pid}\" | Select-Object -First 1) -ne $null"
        );
        command_has_output(
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", &script],
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        false
    }
}

pub fn reap_orphaned_shells() {
    #[cfg(target_os = "linux")]
    linux_reap_orphaned_shells();
}

#[cfg(any(target_os = "macos", windows))]
fn command_has_output(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn linux_reap_orphaned_shells() {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        if orphaned_unit_agent(pid) {
            log::info!("stopping orphaned unit agent shell {pid}");
            unsafe {
                libc::kill(pid as i32, libc::SIGHUP);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn orphaned_unit_agent(pid: u32) -> bool {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    let Some(ppid) = status.lines().find_map(|line| line.strip_prefix("PPid:")) else {
        return false;
    };
    if ppid.trim() != "1" {
        return false;
    }
    let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    env.windows(b"UNIT_AGENT=1".len())
        .any(|window| window == b"UNIT_AGENT=1")
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostIdentity {
    pub computer_name: String,
    pub server_name: String,
    pub network: String,
}

pub fn host_identity() -> HostIdentity {
    let computer_name = computer_name();
    let server_name = server_name();
    HostIdentity {
        network: active_network().unwrap_or_default(),
        computer_name,
        server_name,
    }
}

fn computer_name() -> String {
    #[cfg(windows)]
    if let Some(name) = env_nonempty("COMPUTERNAME") {
        return name;
    }
    #[cfg(target_os = "macos")]
    if let Some(name) = command_stdout("scutil", &["--get", "ComputerName"]) {
        return name;
    }
    server_name()
}

fn server_name() -> String {
    command_stdout("hostname", &[])
        .or_else(|| env_nonempty("HOSTNAME"))
        .or_else(|| env_nonempty("COMPUTERNAME"))
        .unwrap_or_else(|| "this-computer".into())
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn active_network() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(network) = command_stdout("ip", &["-4", "route", "show", "default"])
            .and_then(|text| parse_ip_route(&text))
        {
            return Some(network);
        }
        if let Ok(text) = std::fs::read_to_string("/proc/net/route") {
            if let Some(network) = parse_proc_route(&text) {
                return Some(network);
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        command_stdout("route", &["-n", "get", "default"]).and_then(|text| parse_macos_route(&text))
    }
    #[cfg(windows)]
    {
        command_stdout("route", &["print", "-4"]).and_then(|text| parse_windows_route(&text))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    None
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub fn parse_ip_route(text: &str) -> Option<String> {
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if !parts.contains(&"default") {
            continue;
        }
        let mut iface = None;
        let mut src = None;
        let mut index = 0;
        while index + 1 < parts.len() {
            match parts[index] {
                "dev" => iface = Some(parts[index + 1]),
                "src" => src = Some(parts[index + 1]),
                _ => {}
            }
            index += 1;
        }
        if let Some(iface) = iface {
            return Some(match src {
                Some(address) => format!("{iface} {address}"),
                None => iface.to_string(),
            });
        }
    }
    None
}

pub fn parse_proc_route(text: &str) -> Option<String> {
    for line in text.lines().skip(1) {
        let mut columns = line.split_whitespace();
        let Some(iface) = columns.next() else {
            continue;
        };
        let destination = columns.next().unwrap_or("");
        if destination == "00000000" && !iface.is_empty() {
            return Some(iface.to_string());
        }
    }
    None
}

#[cfg(any(test, target_os = "macos"))]
pub fn parse_macos_route(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("interface:") else {
            continue;
        };
        let name = rest.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(any(test, windows))]
pub fn parse_windows_route(text: &str) -> Option<String> {
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 && parts[0] == "0.0.0.0" && parts[1] == "0.0.0.0" {
            return Some(parts[3].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_real_shell() {
        let shell = detect_shell(None).unwrap();
        assert!(std::path::Path::new(&shell).is_file());
    }

    #[test]
    fn classifies_shell_names() {
        assert_eq!(shell_kind("/bin/bash"), "bash");
        assert_eq!(shell_kind("/bin/zsh"), "zsh");
        assert_eq!(shell_kind("/usr/bin/fish"), "fish");
        assert_eq!(
            shell_kind("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            "powershell"
        );
        assert_eq!(shell_kind("pwsh.exe"), "powershell");
        assert_eq!(shell_kind("cmd.EXE"), "cmd");
    }

    #[test]
    fn reads_the_network_this_computer_uses() {
        let route = "default via 10.0.2.2 dev eth0 proto dhcp src 10.0.2.15 metric 100\n";
        assert_eq!(parse_ip_route(route).as_deref(), Some("eth0 10.0.2.15"));
        let proc_route =
            "Iface\tDestination\tGateway\neth0\t00000000\t0102A8C0\neth0\t0002A8C0\t00000000\n";
        assert_eq!(parse_proc_route(proc_route).as_deref(), Some("eth0"));
        let macos = "  gateway: 192.168.1.1\n  interface: en0\n";
        assert_eq!(parse_macos_route(macos).as_deref(), Some("en0"));
        let windows =
            "          0.0.0.0          0.0.0.0      192.168.1.1     192.168.1.23     25\n";
        assert_eq!(
            parse_windows_route(windows).as_deref(),
            Some("192.168.1.23")
        );
        let host = host_identity();
        assert!(!host.computer_name.is_empty());
        assert!(!host.server_name.is_empty());
    }
}
