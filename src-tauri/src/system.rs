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
}
