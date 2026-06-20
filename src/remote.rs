use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct RemoteFileInfo {
    pub path: String,
    pub size: u64,
    pub mtime: String,
    pub is_dir: bool,
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | '~' | '@'))
    {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

pub fn join_remote(dir: &str, name: &str) -> String {
    let trimmed = dir.trim_end_matches('/');
    if trimmed.is_empty() {
        format!("/{name}")
    } else {
        format!("{trimmed}/{name}")
    }
}

pub fn destination_basename(local: &str) -> Option<String> {
    let trimmed = local.trim_end_matches('/');
    Path::new(trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

pub fn ssh_run(host: &str, remote_cmd: &str) -> io::Result<std::process::Output> {
    Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg("-o")
        .arg("ControlMaster=auto")
        .arg("-o")
        .arg("ControlPath=~/.ssh/snd-%r@%h:%p")
        .arg("-o")
        .arg("ControlPersist=60")
        .arg(host)
        .arg(remote_cmd)
        .output()
}

pub fn stat_remote(host: &str, paths: &[String]) -> Result<Vec<RemoteFileInfo>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let quoted: Vec<String> = paths.iter().map(|p| shell_quote(p)).collect();
    let joined = quoted.join(" ");
    let cmd = format!(
        "stat -c '%n|%s|%y|%F' {joined} 2>/dev/null || stat -f '%N|%z|%Sm|%HT' -t '%Y-%m-%d %H:%M:%S' {joined} 2>/dev/null"
    );
    let out = ssh_run(host, &cmd).map_err(|e| format!("ssh: {e}"))?;

    if out.status.code() == Some(255) {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr.trim();
        return Err(if msg.is_empty() {
            format!("ssh to {host} failed (exit 255)")
        } else {
            format!("ssh to {host} failed: {msg}")
        });
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 {
            continue;
        }
        let path = parts[0].to_string();
        let size: u64 = parts[1].trim().parse().unwrap_or(0);
        let mtime = clean_mtime(parts[2]);
        let kind = parts[3].to_lowercase();
        let is_dir = kind.contains("directory");
        results.push(RemoteFileInfo {
            path,
            size,
            mtime,
            is_dir,
        });
    }
    Ok(results)
}

fn clean_mtime(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(dot) = raw.find('.') {
        let prefix = &raw[..dot];
        let after = &raw[dot..];
        if let Some(space) = after.find(' ') {
            let tz = &after[space + 1..];
            return format!("{prefix} {tz}").trim().to_string();
        }
        return prefix.to_string();
    }
    raw.to_string()
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut idx = 0;
    while v >= 1024.0 && idx < UNITS.len() - 1 {
        v /= 1024.0;
        idx += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[idx])
    } else if v >= 10.0 {
        format!("{v:.1} {}", UNITS[idx])
    } else {
        format!("{v:.2} {}", UNITS[idx])
    }
}

pub fn rm_remote(
    host: &str,
    paths: &[String],
    recursive: bool,
) -> io::Result<std::process::ExitStatus> {
    let quoted: Vec<String> = paths.iter().map(|p| shell_quote(p)).collect();
    let joined = quoted.join(" ");
    let cmd = if recursive {
        format!("rm -rf -- {joined}")
    } else {
        format!("rm -- {joined}")
    };
    Command::new("ssh")
        .arg("-o")
        .arg("ControlMaster=auto")
        .arg("-o")
        .arg("ControlPath=~/.ssh/snd-%r@%h:%p")
        .arg("-o")
        .arg("ControlPersist=60")
        .arg(host)
        .arg(cmd)
        .status()
}

pub fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = io::stdout().flush();
    let stdin = io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim(), "y" | "Y" | "yes" | "YES" | "Yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_keeps_safe_chars_unquoted() {
        assert_eq!(shell_quote("/var/www"), "/var/www");
        assert_eq!(shell_quote("file-1.txt"), "file-1.txt");
        assert_eq!(shell_quote("~/projects"), "~/projects");
    }

    #[test]
    fn shell_quote_wraps_unsafe_chars() {
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote("a$b"), "'a$b'");
    }

    #[test]
    fn shell_quote_escapes_embedded_quote() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_quote_empty() {
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn join_remote_handles_trailing_slash() {
        assert_eq!(join_remote("/var/www", "a.txt"), "/var/www/a.txt");
        assert_eq!(join_remote("/var/www/", "a.txt"), "/var/www/a.txt");
        assert_eq!(join_remote("~", "a.txt"), "~/a.txt");
    }

    #[test]
    fn destination_basename_strips_trailing_slash() {
        assert_eq!(destination_basename("dir/"), Some("dir".to_string()));
        assert_eq!(destination_basename("./a/b.txt"), Some("b.txt".to_string()));
        assert_eq!(destination_basename("/"), None);
    }

    #[test]
    fn format_size_picks_unit() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1024 * 1024), "1.00 MB");
        assert!(format_size(1500).starts_with("1.4"));
        assert!(format_size(1024 * 1024 * 200).ends_with("MB"));
    }

    #[test]
    fn clean_mtime_trims_subsecond() {
        assert_eq!(
            clean_mtime("2025-01-15 10:30:45.000000000 +0000"),
            "2025-01-15 10:30:45 +0000"
        );
        assert_eq!(clean_mtime("2025-01-15 10:30:45"), "2025-01-15 10:30:45");
    }
}
