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

pub fn has_glob(path: &str) -> bool {
    path.contains(['*', '?', '['])
}

pub fn expand_remote_glob(host: &str, pattern: &str) -> Result<Vec<String>, String> {
    let cmd = format!("for p in {pattern}; do [ -e \"$p\" ] && printf '%s\\n' \"$p\"; done");
    let out = ssh_run(host, &cmd).map_err(|e| format!("ssh: {e}"))?;
    if out.status.code() == Some(255) {
        return Err(ssh_error_message(host, &out.stderr));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut paths: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub fn glob_label(pattern: &str, matched: &str) -> String {
    let pc: Vec<&str> = pattern.split('/').collect();
    let mc: Vec<&str> = matched.split('/').collect();
    let is_glob = |s: &&str| s.contains(['*', '?', '[']);
    let (Some(first), Some(last)) = (pc.iter().position(is_glob), pc.iter().rposition(is_glob))
    else {
        return matched.to_string();
    };
    let after_first = pc.len() - 1 - first;
    let after_last = pc.len() - 1 - last;
    if mc.len() > after_first {
        let start = mc.len() - 1 - after_first;
        let end = mc.len() - 1 - after_last;
        if start <= end && end < mc.len() {
            return mc[start..=end].join("/");
        }
    }
    matched
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(matched)
        .to_string()
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

fn ssh_error_message(host: &str, stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let msg = stderr.trim();
    if msg.is_empty() {
        format!("ssh to {host} failed (exit 255)")
    } else {
        format!("ssh to {host} failed: {msg}")
    }
}

fn build_find_cmd(
    base: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
    max_depth: Option<u32>,
) -> String {
    let mut cmd = format!("find {}", shell_quote(base));
    if let Some(d) = max_depth {
        cmd.push_str(&format!(" -maxdepth {d}"));
    }
    if regex {
        let flag = if case_sensitive { "-regex" } else { "-iregex" };
        let mut re = String::new();
        if !pattern.starts_with('^') {
            re.push_str(".*");
        }
        re.push_str(pattern);
        if !pattern.ends_with('$') {
            re.push_str(".*");
        }
        cmd.push_str(&format!(
            " -regextype posix-extended {flag} {}",
            shell_quote(&re)
        ));
    } else {
        let flag = if case_sensitive { "-name" } else { "-iname" };
        let glob = if pattern.contains(['*', '?', '[']) {
            pattern.to_string()
        } else {
            format!("*{pattern}*")
        };
        cmd.push_str(&format!(" {flag} {}", shell_quote(&glob)));
    }
    cmd
}

fn build_grep_cmd(base: &str, pattern: &str, regex: bool, case_sensitive: bool) -> String {
    let mut cmd = String::from("grep -rnI");
    if !case_sensitive {
        cmd.push_str(" -i");
    }
    cmd.push_str(if regex { " -E" } else { " -F" });
    cmd.push_str(&format!(
        " -e {} -- {}",
        shell_quote(pattern),
        shell_quote(base)
    ));
    cmd
}

pub fn find_remote(
    host: &str,
    base: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
    max_depth: Option<u32>,
) -> Result<Vec<String>, String> {
    let cmd = build_find_cmd(base, pattern, regex, case_sensitive, max_depth);
    let out = ssh_run(host, &cmd).map_err(|e| format!("ssh: {e}"))?;
    if out.status.code() == Some(255) {
        return Err(ssh_error_message(host, &out.stderr));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let paths: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if paths.is_empty() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("No such file or directory") {
            let msg = stderr.trim();
            return Err(msg.lines().next().unwrap_or(msg).to_string());
        }
    }
    Ok(paths)
}

pub fn grep_remote(
    host: &str,
    base: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
) -> Result<Vec<String>, String> {
    let cmd = build_grep_cmd(base, pattern, regex, case_sensitive);
    let out = ssh_run(host, &cmd).map_err(|e| format!("ssh: {e}"))?;
    match out.status.code() {
        Some(0) => {}
        Some(1) => return Ok(Vec::new()),
        Some(255) => return Err(ssh_error_message(host, &out.stderr)),
        _ if out.stdout.is_empty() => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg = stderr.trim();
            return Err(if msg.is_empty() {
                format!("grep failed on {host}")
            } else {
                msg.lines().next().unwrap_or(msg).to_string()
            });
        }
        _ => {}
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
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
        return Err(ssh_error_message(host, &out.stderr));
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

pub fn cat_remote(host: &str, paths: &[String]) -> io::Result<std::process::ExitStatus> {
    let quoted: Vec<String> = paths.iter().map(|p| shell_quote(p)).collect();
    let cmd = format!("cat -- {}", quoted.join(" "));
    ssh_command(host, &cmd).status()
}

pub fn ls_remote(host: &str, path: &str) -> io::Result<std::process::ExitStatus> {
    let cmd = format!("ls -lhA -- {}", shell_quote(path));
    ssh_command(host, &cmd).status()
}

fn ssh_command(host: &str, remote_cmd: &str) -> Command {
    let mut c = Command::new("ssh");
    c.arg("-o")
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
        .arg(remote_cmd);
    c
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

    #[test]
    fn build_find_cmd_default_is_case_insensitive_substring() {
        assert_eq!(
            build_find_cmd("/opt/app", "essentials", false, false, None),
            "find /opt/app -iname '*essentials*'"
        );
    }

    #[test]
    fn build_find_cmd_case_sensitive_uses_name() {
        assert_eq!(
            build_find_cmd("/opt/app", "Ess", false, true, None),
            "find /opt/app -name '*Ess*'"
        );
    }

    #[test]
    fn build_find_cmd_glob_pattern_passed_through() {
        assert_eq!(
            build_find_cmd("/opt/app", "World*.jar", false, false, None),
            "find /opt/app -iname 'World*.jar'"
        );
    }

    #[test]
    fn build_find_cmd_regex_wraps_and_sets_regextype() {
        assert_eq!(
            build_find_cmd("/opt/app", "world(edit|guard)", true, false, None),
            "find /opt/app -regextype posix-extended -iregex '.*world(edit|guard).*'"
        );
    }

    #[test]
    fn build_find_cmd_regex_respects_anchors() {
        assert_eq!(
            build_find_cmd("/opt/app", "foo\\.jar$", true, false, None),
            "find /opt/app -regextype posix-extended -iregex '.*foo\\.jar$'"
        );
        assert_eq!(
            build_find_cmd("/opt/app", "^/opt/app/x", true, true, None),
            "find /opt/app -regextype posix-extended -regex '^/opt/app/x.*'"
        );
    }

    #[test]
    fn build_find_cmd_applies_max_depth() {
        assert_eq!(
            build_find_cmd("/", "x", false, false, Some(3)),
            "find / -maxdepth 3 -iname '*x*'"
        );
    }

    #[test]
    fn build_find_cmd_quotes_base_with_spaces() {
        assert_eq!(
            build_find_cmd("/opt/my app", "x", false, false, None),
            "find '/opt/my app' -iname '*x*'"
        );
    }

    #[test]
    fn build_grep_cmd_default_fixed_case_insensitive() {
        assert_eq!(
            build_grep_cmd("/opt/app", "db.host", false, false),
            "grep -rnI -i -F -e db.host -- /opt/app"
        );
    }

    #[test]
    fn build_grep_cmd_regex_and_case_sensitive() {
        assert_eq!(
            build_grep_cmd("/opt/app", "db\\.host", true, true),
            "grep -rnI -E -e 'db\\.host' -- /opt/app"
        );
    }

    #[test]
    fn has_glob_detects_metachars() {
        assert!(has_glob("app-*_*/plugins"));
        assert!(has_glob("a?b"));
        assert!(has_glob("srv/[abc]/x"));
        assert!(!has_glob("/opt/app/plugins"));
        assert!(!has_glob("~/plain/path"));
    }

    #[test]
    fn glob_label_takes_varying_segment_with_tilde_prefix() {
        assert_eq!(
            glob_label(
                "~/app/instances/app-*_*/plugins",
                "/home/deploy/app/instances/app-1_a1b2c3d4/plugins"
            ),
            "app-1_a1b2c3d4"
        );
    }

    #[test]
    fn glob_label_spans_multiple_glob_segments() {
        assert_eq!(
            glob_label("~/srv/*/data-*/plugins", "/home/u/srv/alpha/data-3/plugins"),
            "alpha/data-3"
        );
    }

    #[test]
    fn glob_label_trailing_glob_uses_final_segment() {
        assert_eq!(glob_label("/var/log/*.log", "/var/log/app.log"), "app.log");
    }

    #[test]
    fn glob_label_no_glob_returns_whole_match() {
        assert_eq!(glob_label("/opt/app", "/opt/app"), "/opt/app");
    }

    #[test]
    fn build_grep_cmd_quotes_dangerous_pattern() {
        assert_eq!(
            build_grep_cmd("/opt/app", "a b; rm", false, false),
            "grep -rnI -i -F -e 'a b; rm' -- /opt/app"
        );
    }
}
