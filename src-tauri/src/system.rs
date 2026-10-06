use std::fs;
use std::path::Path;

use crate::error::AppError;

pub fn home_dir() -> String {
    dirs::home_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".into())
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
    candidates.push("/bin/bash".into());
    candidates.push("/usr/bin/bash".into());
    candidates.push("/bin/zsh".into());
    candidates.push("/bin/sh".into());
    for candidate in candidates {
        if Path::new(&candidate).is_file() {
            return Ok(candidate);
        }
    }
    Err(AppError::shell("No usable shell was found on this system."))
}

pub fn shell_kind(shell: &str) -> &'static str {
    let name = Path::new(shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(shell);
    match name {
        "zsh" => "zsh",
        "fish" => "fish",
        "bash" => "bash",
        _ => "other",
    }
}

pub fn has_child_processes(pid: u32) -> bool {
    let path = format!("/proc/{pid}/task/{pid}/children");
    fs::read_to_string(path)
        .map(|text| text.split_whitespace().any(|part| !part.is_empty()))
        .unwrap_or(false)
}

pub fn reap_orphaned_shells() {
    let Ok(entries) = fs::read_dir("/proc") else {
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

fn orphaned_unit_agent(pid: u32) -> bool {
    let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    let Some(ppid) = status.lines().find_map(|line| line.strip_prefix("PPid:")) else {
        return false;
    };
    if ppid.trim() != "1" {
        return false;
    }
    let Ok(env) = fs::read(format!("/proc/{pid}/environ")) else {
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
        assert!(Path::new(&shell).is_file());
    }
}
