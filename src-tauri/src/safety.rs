/// Conservative detector for clearly destructive local commands.
/// It is intentionally incomplete: history expansion, aliases, and commands
/// already sitting in the shell's line editor cannot be seen perfectly.
pub fn is_dangerous(command: &str) -> bool {
    let normalized = normalize(command);
    if normalized.is_empty() {
        return false;
    }
    fork_bomb(&normalized)
        || filesystem_destroy(&normalized)
        || disk_destroy(&normalized)
        || power_command(&normalized)
        || package_removal(&normalized)
        || recursive_force_remove(&normalized)
        || windows_destroy(&normalized)
}

fn normalize(command: &str) -> String {
    let mut line = command.trim().to_string();
    if let Some(stripped) = line.strip_prefix("#") {
        let _ = stripped;
        return String::new();
    }
    loop {
        let next = strip_wrapper(&line);
        if next == line {
            break;
        }
        line = next;
    }
    collapse_ws(&line).to_lowercase()
}

fn strip_wrapper(line: &str) -> String {
    let trimmed = line.trim();
    for prefix in ["sudo ", "doas ", "command "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return strip_sudo_flags(rest);
        }
    }
    if let Some(rest) = strip_env_assignment(trimmed) {
        return rest;
    }
    trimmed.to_string()
}

fn strip_sudo_flags(rest: &str) -> String {
    let mut parts = rest.split_whitespace().peekable();
    while let Some(part) = parts.peek().copied() {
        if part == "-u" || part == "-g" || part == "--user" || part == "--group" {
            parts.next();
            parts.next();
            continue;
        }
        if part.starts_with('-') {
            parts.next();
            continue;
        }
        break;
    }
    parts.collect::<Vec<_>>().join(" ")
}

fn strip_env_assignment(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    if looks_like_env_assignment(first) {
        let rest = parts.collect::<Vec<_>>().join(" ");
        if rest.is_empty() {
            return Some(String::new());
        }
        return Some(rest);
    }
    None
}

fn looks_like_env_assignment(token: &str) -> bool {
    let mut chars = token.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    let Some((name, value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !value.contains(' ')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn collapse_ws(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fork_bomb(line: &str) -> bool {
    line.replace(' ', "").contains(":(){:|:&};:") || line.contains(":(){ :|:& };:")
}

fn power_command(line: &str) -> bool {
    let cmd = first_word(line);
    matches!(cmd, "shutdown" | "reboot" | "poweroff" | "halt")
        || line.starts_with("systemctl poweroff")
        || line.starts_with("systemctl reboot")
        || line.starts_with("systemctl halt")
        || line.starts_with("init 0")
        || line.starts_with("init 6")
}

fn disk_destroy(line: &str) -> bool {
    let cmd = first_word(line);
    if cmd == "mkfs" || cmd.starts_with("mkfs.") || cmd == "wipefs" {
        return true;
    }
    if cmd == "dd" && (line.contains("if=") || line.contains("of=/dev/")) {
        return true;
    }
    if cmd == "shred" && line.contains("/dev/") {
        return true;
    }
    if line.starts_with("diskutil erasedisk") || line.starts_with("diskutil erasevolume") {
        return true;
    }
    if (line.contains("> /dev/sd") || line.contains("> /dev/nvme") || line.contains("> /dev/mmc"))
        && !line.starts_with("echo ")
    {
        return true;
    }
    false
}

fn filesystem_destroy(line: &str) -> bool {
    (line.starts_with("chmod -r 777 /") || line.starts_with("chmod -r 000 /"))
        || (line.starts_with("chown -r ") && (line.ends_with(" /") || line.contains(" / ")))
}

fn package_removal(line: &str) -> bool {
    line.starts_with("apt remove")
        || line.starts_with("apt purge")
        || line.starts_with("apt-get remove")
        || line.starts_with("apt-get purge")
        || line.starts_with("dnf remove")
        || line.starts_with("yum remove")
        || line.starts_with("pacman -r")
        || line.starts_with("snap remove")
}

fn recursive_force_remove(line: &str) -> bool {
    if !line.starts_with("rm ") {
        return false;
    }
    let after = line.trim_start_matches("rm ").trim();
    let mut force = false;
    let mut recursive = false;
    let mut paths = Vec::new();
    for token in after.split_whitespace() {
        if token.starts_with('-') && !token.starts_with("--") {
            if token.contains('r') || token.contains('R') {
                recursive = true;
            }
            if token.contains('f') {
                force = true;
            }
            continue;
        }
        if token == "--recursive" || token == "-r" || token == "-R" {
            recursive = true;
            continue;
        }
        if token == "--force" || token == "-f" {
            force = true;
            continue;
        }
        paths.push(token);
    }
    if !(force && recursive) {
        return false;
    }
    paths.iter().any(|path| dangerous_rm_path(path))
}

fn dangerous_rm_path(path: &str) -> bool {
    matches!(
        path,
        "/" | "/*"
            | "/."
            | "~"
            | "~/"
            | "~/*"
            | "$home"
            | "$home/"
            | "${home}"
            | "."
            | "./*"
            | "*"
            | "/home"
            | "/home/*"
            | "/etc"
            | "/etc/*"
            | "/usr"
            | "/usr/*"
            | "/bin"
            | "/bin/*"
            | "/boot"
            | "/boot/*"
            | "/var"
            | "/var/*"
            | "/opt"
            | "/opt/*"
            | "/root"
            | "/root/*"
    )
}

fn windows_destroy(line: &str) -> bool {
    if line.starts_with("stop-computer")
        || line.starts_with("restart-computer")
        || line.starts_with("format-volume")
        || line.starts_with("clear-disk")
    {
        return true;
    }
    if line.starts_with("format ") {
        let rest = line.trim_start_matches("format ").trim_start();
        let token = rest.split_whitespace().next().unwrap_or("");
        if token.len() >= 2
            && token.as_bytes()[1] == b':'
            && token.as_bytes()[0].is_ascii_alphabetic()
        {
            return true;
        }
    }
    remove_item_root(line)
}

fn remove_item_root(line: &str) -> bool {
    let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
    if !matches!(cmd, "remove-item" | "ri" | "del" | "erase" | "rmdir" | "rd") {
        return false;
    }
    let recursive =
        rest.contains("-recurse") || rest.contains("/s") || matches!(cmd, "rmdir" | "rd");
    let force = rest.contains("-force")
        || rest.contains("/f")
        || rest.contains("/q")
        || matches!(cmd, "del" | "erase");
    if !recursive || !force {
        return false;
    }
    rest.split_whitespace().any(dangerous_windows_path)
}

fn dangerous_windows_path(token: &str) -> bool {
    let token = token.trim_matches('"').trim_matches('\'');
    matches!(
        token,
        "c:\\"
            | "c:/"
            | "c:\\*"
            | "c:/*"
            | "\\"
            | "/"
            | "~"
            | "~\\"
            | "~/"
            | "$env:userprofile"
            | "$env:systemroot"
            | "c:\\windows"
            | "c:/windows"
            | "c:\\users"
            | "c:/users"
    )
}

fn first_word(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAction {
    Forward(String),
    Confirm { forward: String, command: String },
    Held,
}

#[derive(Debug, Default)]
pub struct LineGuard {
    line: String,
    pending: Option<String>,
    skipping_escape: bool,
}

impl LineGuard {
    pub fn push(&mut self, data: &str) -> GuardAction {
        if self.pending.is_some() {
            if data.chars().any(|ch| ch == '\u{3}') {
                self.pending = None;
                self.line.clear();
                self.skipping_escape = false;
                return GuardAction::Forward("\u{3}".into());
            }
            return GuardAction::Held;
        }

        let mut forward = String::new();
        for ch in data.chars() {
            if self.skipping_escape {
                forward.push(ch);
                if ch == '\u{1b}' {
                    continue;
                }
                if ch.is_ascii_alphabetic() || ch == '~' {
                    self.skipping_escape = false;
                }
                continue;
            }
            match ch {
                '\r' | '\n' => {
                    let command = self.line.trim().to_string();
                    if is_dangerous(&command) {
                        self.pending = Some(command.clone());
                        return GuardAction::Confirm { forward, command };
                    }
                    forward.push(ch);
                    self.line.clear();
                }
                '\u{7f}' | '\u{8}' => {
                    forward.push(ch);
                    self.line.pop();
                }
                '\u{3}' | '\u{15}' => {
                    forward.push(ch);
                    self.line.clear();
                }
                '\u{1b}' => {
                    forward.push(ch);
                    self.skipping_escape = true;
                }
                c if c.is_control() => {
                    forward.push(c);
                }
                c => {
                    forward.push(c);
                    self.line.push(c);
                }
            }
        }
        GuardAction::Forward(forward)
    }

    pub fn approve(&mut self) -> Option<String> {
        self.pending.take().map(|_| {
            self.line.clear();
            "\r".to_string()
        })
    }

    pub fn deny(&mut self) -> Option<String> {
        self.pending.take().map(|_| {
            self.line.clear();
            "\u{15}".to_string()
        })
    }

    pub fn pending(&self) -> Option<&str> {
        self.pending.as_deref()
    }
}

pub fn danger_in_json(value: &serde_json::Value) -> Option<String> {
    let mut found = None;
    walk_json(value, &mut found);
    found
}

fn walk_json(value: &serde_json::Value, found: &mut Option<String>) {
    if found.is_some() {
        return;
    }
    match value {
        serde_json::Value::String(text) => {
            if is_dangerous(text) {
                *found = Some(text.clone());
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_json(item, found);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                walk_json(item, found);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_root_removal_and_wrappers() {
        assert!(is_dangerous("rm -rf /"));
        assert!(is_dangerous("rm -rf /*"));
        assert!(is_dangerous("sudo rm -rf /"));
        assert!(is_dangerous("FOO=1 sudo -u root rm -fr ~"));
        assert!(is_dangerous("rm -rf $HOME"));
        assert!(is_dangerous("rm -rf ."));
        assert!(is_dangerous("rm -rf *"));
    }

    #[test]
    fn allows_ordinary_cleanup() {
        assert!(!is_dangerous("rm -rf node_modules"));
        assert!(!is_dangerous("rm -rf ./target"));
        assert!(!is_dangerous("rm -rf /tmp/unit-agent-test"));
        assert!(!is_dangerous("echo rm -rf /"));
        assert!(!is_dangerous("ls /"));
        assert!(!is_dangerous(""));
    }

    #[test]
    fn flags_disk_power_packages_and_fork_bomb() {
        assert!(is_dangerous("mkfs.ext4 /dev/sda"));
        assert!(is_dangerous("dd if=/dev/zero of=/dev/sda"));
        assert!(is_dangerous("shutdown -h now"));
        assert!(is_dangerous("reboot"));
        assert!(is_dangerous("apt purge vim"));
        assert!(is_dangerous(":(){ :|:& };:"));
        assert!(is_dangerous("chmod -R 777 /"));
        assert!(is_dangerous("diskutil eraseDisk APFS Disk disk0"));
    }

    #[test]
    fn flags_windows_disk_and_removal() {
        assert!(is_dangerous("Stop-Computer -Force"));
        assert!(is_dangerous("Restart-Computer"));
        assert!(is_dangerous("Format C:"));
        assert!(is_dangerous("Format-Volume -DriveLetter C"));
        assert!(is_dangerous(r"Remove-Item -Recurse -Force C:\"));
        assert!(is_dangerous(r"del /f /s C:\Windows"));
        assert!(!is_dangerous(r"Remove-Item .\notes.txt"));
        assert!(!is_dangerous(r"del report.txt"));
        assert!(!is_dangerous("echo Format C:"));
    }

    #[test]
    fn guard_holds_newline_until_decision() {
        let mut guard = LineGuard::default();
        match guard.push("rm -rf /\r") {
            GuardAction::Confirm { forward, command } => {
                assert!(!forward.contains('\r'));
                assert_eq!(command, "rm -rf /");
            }
            other => panic!("expected confirm, got {other:?}"),
        }
        assert!(matches!(guard.push("ls\r"), GuardAction::Held));
        assert_eq!(guard.deny().as_deref(), Some("\u{15}"));
        match guard.push("echo ok\r") {
            GuardAction::Forward(data) => assert!(data.contains('\r')),
            other => panic!("expected forward, got {other:?}"),
        }
    }

    #[test]
    fn json_scanner_finds_nested_command() {
        let value = serde_json::json!({"tool": {"command": "mkfs.ext4 /dev/sdb"}});
        assert!(danger_in_json(&value).is_some());
        assert!(danger_in_json(&serde_json::json!({"command": "echo hi"})).is_none());
    }
}
