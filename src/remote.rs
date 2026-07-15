use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::transfer::TransferOptions;

static TRANSFER_OPTIONS: OnceLock<TransferOptions> = OnceLock::new();

pub fn set_transfer_options(options: TransferOptions) {
    let _ = TRANSFER_OPTIONS.set(options);
}

fn apply_ssh_options(command: &mut Command) {
    if let Some(options) = TRANSFER_OPTIONS.get() {
        options.apply_ssh(command);
    }
}

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

fn remote_path_quote(path: &str) -> String {
    if path == "~" {
        return "\"$HOME\"".to_string();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return format!("\"$HOME\"/{}", shell_quote(rest));
    }
    if let Some(tilde_path) = path.strip_prefix('~') {
        let (user, rest) = tilde_path
            .split_once('/')
            .map_or((tilde_path, None), |(user, rest)| (user, Some(rest)));
        if !user.is_empty()
            && user
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return match rest {
                Some(rest) => format!("~{user}/{}", shell_quote(rest)),
                None => format!("~{user}"),
            };
        }
    }
    shell_quote(path)
}

pub fn has_glob(path: &str) -> bool {
    path.contains(['*', '?', '['])
}

pub fn has_unescaped_glob(path: &str) -> bool {
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && chars
                .peek()
                .is_some_and(|next| matches!(*next, '\\' | '*' | '?' | '[' | ']'))
        {
            let _ = chars.next();
            continue;
        }
        if matches!(c, '*' | '?' | '[') {
            return true;
        }
    }
    false
}

pub fn unescape_glob_literals(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && chars
                .peek()
                .is_some_and(|next| matches!(*next, '\\' | '*' | '?' | '[' | ']'))
        {
            out.push(chars.next().expect("peeked escaped glob character"));
        } else {
            out.push(c);
        }
    }
    out
}

pub fn scp_literal_remote_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn shell_glob_word(pattern: &str) -> Result<String, String> {
    if pattern.contains(['\n', '\r']) {
        return Err("glob paths cannot contain newlines".to_string());
    }

    let mut out = String::with_capacity(pattern.len());
    for c in pattern.chars() {
        if c.is_ascii_alphanumeric()
            || matches!(
                c,
                '/' | '_'
                    | '-'
                    | '.'
                    | '~'
                    | '@'
                    | ':'
                    | '%'
                    | '+'
                    | '='
                    | ','
                    | '!'
                    | '^'
                    | '*'
                    | '?'
                    | '['
                    | ']'
            )
        {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    Ok(out)
}

pub fn expand_remote_glob(host: &str, pattern: &str) -> Result<Vec<String>, String> {
    let pattern = shell_glob_word(pattern)?;
    let cmd = format!("for p in {pattern}; do [ -d \"$p\" ] && printf '%s\\n' \"$p\"; done");
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
    let mut command = Command::new("ssh");
    apply_ssh_options(&mut command);
    command
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
        .arg("--")
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
    let mut cmd = format!("find {}", remote_path_quote(base));
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

fn build_grep_cmd(
    base: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
    color: bool,
) -> String {
    let mut cmd = String::from("grep -rnI");
    if color {
        cmd.push_str(" --color=always");
    }
    if !case_sensitive {
        cmd.push_str(" -i");
    }
    cmd.push_str(if regex { " -E" } else { " -F" });
    cmd.push_str(&format!(
        " -e {} -- {}",
        shell_quote(pattern),
        remote_path_quote(base)
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
    color: bool,
) -> Result<Vec<String>, String> {
    let cmd = build_grep_cmd(base, pattern, regex, case_sensitive, color);
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
    let quoted: Vec<String> = paths.iter().map(|p| remote_path_quote(p)).collect();
    let joined = quoted.join(" ");
    let cmd = format!(
        concat!(
            "if stat -c '%s' / >/dev/null 2>&1; then style=gnu; else style=bsd; fi; ",
            "i=0; for p in {joined}; do ",
            "if [ -e \"$p\" ] || [ -L \"$p\" ]; then ",
            "if [ \"$style\" = gnu ]; then ",
            "data=$(stat -c '%s|%y|%F' \"$p\" 2>/dev/null); ",
            "else data=$(stat -f '%z|%Sm|%HT' -t '%Y-%m-%d %H:%M:%S' \"$p\" 2>/dev/null); fi; ",
            "[ -n \"$data\" ] && printf '%s|%s\\n' \"$i\" \"$data\"; ",
            "fi; i=$((i + 1)); done"
        ),
        joined = joined
    );
    let out = ssh_run(host, &cmd).map_err(|e| format!("ssh: {e}"))?;

    if out.status.code() == Some(255) {
        return Err(ssh_error_message(host, &out.stderr));
    }

    Ok(parse_stat_output(
        paths,
        &String::from_utf8_lossy(&out.stdout),
    ))
}

fn parse_stat_output(paths: &[String], stdout: &str) -> Vec<RemoteFileInfo> {
    let mut results = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 {
            continue;
        }
        let Ok(index) = parts[0].parse::<usize>() else {
            continue;
        };
        let Some(path) = paths.get(index).cloned() else {
            continue;
        };
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
    results
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
    let quoted: Vec<String> = paths.iter().map(|p| remote_path_quote(p)).collect();
    let joined = quoted.join(" ");
    let cmd = if recursive {
        format!("rm -rf -- {joined}")
    } else {
        format!("rm -- {joined}")
    };
    let mut command = Command::new("ssh");
    apply_ssh_options(&mut command);
    command
        .arg("-o")
        .arg("ControlMaster=auto")
        .arg("-o")
        .arg("ControlPath=~/.ssh/snd-%r@%h:%p")
        .arg("-o")
        .arg("ControlPersist=60")
        .arg("--")
        .arg(host)
        .arg(cmd)
        .status()
}

fn build_cat_cmd(paths: &[String]) -> String {
    let quoted: Vec<String> = paths.iter().map(|p| remote_path_quote(p)).collect();
    format!("cat -- {}", quoted.join(" "))
}

fn local_highlighter() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in ["bat", "batcat"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            {
                let candidate = dir.join(format!("{name}.exe"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub fn cat_remote(
    host: &str,
    paths: &[String],
    color: bool,
) -> io::Result<std::process::ExitStatus> {
    let cmd = build_cat_cmd(paths);
    let Some(highlighter) = color.then(local_highlighter).flatten() else {
        return ssh_command(host, &cmd).status();
    };

    let mut ssh = ssh_command(host, &cmd).stdout(Stdio::piped()).spawn()?;
    let ssh_stdout = ssh
        .stdout
        .take()
        .expect("ssh stdout is piped for local highlighting");
    let mut bat = Command::new(highlighter);
    bat.args(["--color=always", "--style=plain", "--paging=never"]);
    if let Some(path) = paths.first() {
        bat.arg("--file-name").arg(path);
    }
    let bat_status = bat.arg("-").stdin(ssh_stdout).status()?;
    let ssh_status = ssh.wait()?;
    if ssh_status.success() {
        Ok(bat_status)
    } else {
        Ok(ssh_status)
    }
}

fn build_ls_cmd(path: &str) -> String {
    format!("ls -lhA -- {}", remote_path_quote(path))
}

fn colorize_ls_line(line: &str) -> String {
    if line.starts_with("total ") {
        return format!("\x1b[2m{line}\x1b[0m");
    }
    let mut spans = Vec::new();
    let mut start = None;
    for (index, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                spans.push((token_start, index));
                if spans.len() == 9 {
                    break;
                }
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if spans.len() < 9
        && let Some(token_start) = start
    {
        spans.push((token_start, line.len()));
    }
    if spans.len() < 9 {
        return format!("\x1b[37m{line}\x1b[0m");
    }

    const FIELD_COLORS: [&str; 8] = ["36", "2", "33", "35", "32", "2", "2", "2"];
    let mut out = String::with_capacity(line.len() + 100);
    let mut cursor = 0;
    for ((field_start, field_end), color) in spans.iter().take(8).zip(FIELD_COLORS) {
        out.push_str(&line[cursor..*field_start]);
        out.push_str("\x1b[");
        out.push_str(color);
        out.push('m');
        out.push_str(&line[*field_start..*field_end]);
        out.push_str("\x1b[0m");
        cursor = *field_end;
    }
    let name_start = spans[8].0;
    out.push_str(&line[cursor..name_start]);
    let name_color = match line.as_bytes().first() {
        Some(b'd') => "1;34",
        Some(b'l') => "1;36",
        _ if line
            .as_bytes()
            .get(1..10)
            .is_some_and(|mode| mode.contains(&b'x')) =>
        {
            "1;32"
        }
        _ => "1;37",
    };
    out.push_str("\x1b[");
    out.push_str(name_color);
    out.push('m');
    out.push_str(&line[name_start..]);
    out.push_str("\x1b[0m");
    out
}

fn colorize_ls_output(output: &str) -> String {
    let mut colored = output
        .lines()
        .map(colorize_ls_line)
        .collect::<Vec<_>>()
        .join("\n");
    if output.ends_with('\n') {
        colored.push('\n');
    }
    colored
}

pub fn ls_remote(host: &str, path: &str, color: bool) -> io::Result<std::process::ExitStatus> {
    let cmd = build_ls_cmd(path);
    if !color {
        return ssh_command(host, &cmd).status();
    }
    let output = ssh_command(host, &cmd).output()?;
    io::stdout()
        .write_all(colorize_ls_output(&String::from_utf8_lossy(&output.stdout)).as_bytes())?;
    io::stderr().write_all(&output.stderr)?;
    Ok(output.status)
}

fn ssh_command(host: &str, remote_cmd: &str) -> Command {
    let mut c = Command::new("ssh");
    apply_ssh_options(&mut c);
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
        .arg("--")
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
        assert_eq!(
            remote_path_quote("~/hello world"),
            "\"$HOME\"/'hello world'"
        );
        assert_eq!(remote_path_quote("~deploy/a b"), "~deploy/'a b'");
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
    fn shell_glob_word_preserves_globs_and_escapes_shell_syntax() {
        assert_eq!(
            shell_glob_word("/srv/app-*; touch /tmp/pwn").unwrap(),
            "/srv/app-*\\;\\ touch\\ /tmp/pwn"
        );
        assert_eq!(
            shell_glob_word("~/instances/app-[12]?/plugins").unwrap(),
            "~/instances/app-[12]?/plugins"
        );
        assert_eq!(
            shell_glob_word("~/instances/app-[!2]/plugins").unwrap(),
            "~/instances/app-[!2]/plugins"
        );
        assert!(shell_glob_word("/tmp/*\nnext").is_err());
    }

    #[test]
    fn escaped_download_globs_are_treated_as_literal_characters() {
        assert!(has_unescaped_glob("*.log"));
        assert!(has_unescaped_glob("report[12].txt"));
        assert!(!has_unescaped_glob(r"report\[1\].txt"));
        assert_eq!(unescape_glob_literals(r"report\[1\].txt"), "report[1].txt");
        assert_eq!(
            scp_literal_remote_path("/srv/report[1].txt"),
            r"/srv/report\[1\].txt"
        );
    }

    #[test]
    fn stat_output_uses_request_index_instead_of_filename_delimiters() {
        let paths = vec!["/tmp/a|b".to_string()];
        let infos = parse_stat_output(
            &paths,
            "0|123|2026-07-15 12:00:00.000000000 +0000|regular file\n",
        );
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].path, "/tmp/a|b");
        assert_eq!(infos[0].size, 123);
        assert!(!infos[0].is_dir);
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
            build_grep_cmd("/opt/app", "db.host", false, false, false),
            "grep -rnI -i -F -e db.host -- /opt/app"
        );
    }

    #[test]
    fn build_grep_cmd_regex_and_case_sensitive() {
        assert_eq!(
            build_grep_cmd("/opt/app", "db\\.host", true, true, false),
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
            build_grep_cmd("/opt/app", "a b; rm", false, false, false),
            "grep -rnI -i -F -e 'a b; rm' -- /opt/app"
        );
    }

    #[test]
    fn color_commands_force_ansi_only_when_requested() {
        assert_eq!(
            build_grep_cmd("/opt/app", "needle", false, false, true),
            "grep -rnI --color=always -i -F -e needle -- /opt/app"
        );
        assert_eq!(build_ls_cmd("/opt/app"), "ls -lhA -- /opt/app");
        assert_eq!(
            build_cat_cmd(&["/opt/app/config.yml".to_string()]),
            "cat -- /opt/app/config.yml"
        );
    }

    #[test]
    fn local_ls_colorizer_colors_every_filename_and_preserves_spaces() {
        let input = concat!(
            "total 8\n",
            "-rw-r--r-- 1 deploy deploy 123 Jul 15 12:00 plain file.txt\n",
            "drwxr-xr-x 2 deploy deploy 4.0K Jul 15 12:00 plugins\n"
        );
        let output = colorize_ls_output(input);
        assert!(output.contains("\x1b[1;37mplain file.txt\x1b[0m"));
        assert!(output.contains("\x1b[1;34mplugins\x1b[0m"));
        assert!(output.contains("\x1b[33mdeploy\x1b[0m"));
        assert!(output.ends_with('\n'));
    }
}
