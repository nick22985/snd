use std::process::Command;
use std::time::Duration;

use clap::{Parser, Subcommand};
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};
use clap_complete::Shell;

use crate::config::load_servers;
use crate::ssh::parse_ssh_hosts;

fn fuzzy_match(pattern: &str, target: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    for c in target.chars() {
        if let Some(&p) = pattern_chars.peek() {
            if c.eq_ignore_ascii_case(&p) {
                pattern_chars.next();
            }
        } else {
            return true;
        }
    }
    pattern_chars.peek().is_none()
}

fn complete_server_alias(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_str().unwrap_or("");
    load_servers()
        .into_iter()
        .filter(|(alias, srv)| {
            current.is_empty() || fuzzy_match(current, alias) || fuzzy_match(current, &srv.host)
        })
        .map(|(alias, srv)| {
            let default = srv.default_target().unwrap_or_else(|| srv.host.clone());
            let help = if srv.paths.len() > 1 {
                format!("{default} (+{} paths)", srv.paths.len() - 1)
            } else {
                default
            };
            CompletionCandidate::new(&alias).help(Some(clap::builder::StyledStr::from(help)))
        })
        .collect()
}

fn complete_path_alias(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_str().unwrap_or("");
    let Some(server) = extract_server_arg() else {
        return Vec::new();
    };
    let servers = load_servers();
    let Some(srv) = servers.get(&server) else {
        return Vec::new();
    };
    srv.paths
        .iter()
        .filter(|(k, _)| current.is_empty() || fuzzy_match(current, k))
        .map(|(k, v)| {
            let help = if k == &srv.default {
                format!("{v} (default)")
            } else {
                v.clone()
            };
            CompletionCandidate::new(k).help(Some(clap::builder::StyledStr::from(help)))
        })
        .collect()
}

fn complete_ssh_target(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_str().unwrap_or("");
    let mut candidates = Vec::new();
    for h in parse_ssh_hosts() {
        let target = h.display_target();
        let matches = current.is_empty()
            || fuzzy_match(current, &h.alias)
            || h.hostname
                .as_deref()
                .is_some_and(|hn| fuzzy_match(current, hn))
            || h.user.as_deref().is_some_and(|u| fuzzy_match(current, u))
            || fuzzy_match(current, &target);
        if !matches {
            continue;
        }
        candidates.push(
            CompletionCandidate::new(&h.alias)
                .help(Some(clap::builder::StyledStr::from(target.clone()))),
        );
        if target != h.alias {
            candidates.push(CompletionCandidate::new(&target).help(Some(
                clap::builder::StyledStr::from(format!("({})", h.alias)),
            )));
        }
    }
    candidates
}

fn subcommand_words() -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .skip_while(|a| *a != "--")
        .skip(2)
        .map(|s| s.to_string())
        .collect()
}

fn extract_host_for_path_completion() -> Option<String> {
    let words = subcommand_words();
    for (i, w) in words.iter().enumerate() {
        match w.as_str() {
            "add" if i + 2 < words.len() => return Some(words[i + 2].clone()),
            "add-path" | "edit-path" if i + 1 < words.len() => {
                let server = &words[i + 1];
                let servers = load_servers();
                if let Some(s) = servers.get(server) {
                    return Some(s.host.clone());
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_server_arg() -> Option<String> {
    let words = subcommand_words();
    for (i, w) in words.iter().enumerate() {
        match w.as_str() {
            "remove-path" | "rm-path" | "set-default" | "edit-path" if i + 1 < words.len() => {
                return Some(words[i + 1].clone());
            }
            _ => {}
        }
    }
    None
}

fn cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("snd")
}

fn cache_key(host: &str, dir: &str) -> String {
    format!(
        "{}-{}",
        host.replace(['/', '@', ':'], "_"),
        dir.replace('/', "_")
    )
}

fn spawn_remote_ls(host: &str, remote_dir: &str) {
    let cache = cache_dir();
    let _ = std::fs::create_dir_all(&cache);
    let key = cache_key(host, remote_dir);
    let cache_file = cache.join(&key);
    let lock_file = cache.join(format!("{key}.lock"));

    if lock_file.exists()
        && let Ok(meta) = lock_file.metadata()
        && let Ok(Ok(age)) = meta.modified().map(|m| m.elapsed())
        && age < Duration::from_secs(30)
    {
        return;
    }

    let _ = std::fs::write(&lock_file, "");

    let cache_path = cache_file.to_string_lossy().to_string();
    let lock_path = lock_file.to_string_lossy().to_string();

    let _ = Command::new("sh")
        .args([
            "-c",
            &format!(
                concat!(
                    "nohup sh -c '",
                    "ssh -o BatchMode=yes -o ConnectTimeout=3",
                    " -o StrictHostKeyChecking=accept-new",
                    " -o ControlMaster=auto",
                    " -o \"ControlPath=~/.ssh/snd-%r@%h:%p\"",
                    " -o ControlPersist=60",
                    " {host} '\"'\"'cd {remote_dir} && pwd && ls -1p'\"'\"'",
                    " > {cache} 2>/dev/null; rm -f {lock}",
                    "' >/dev/null 2>&1 &",
                ),
                host = host,
                remote_dir = remote_dir,
                cache = cache_path,
                lock = lock_path,
            ),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn extract_main_server_arg() -> Option<String> {
    let words = subcommand_words();
    let first = words.first()?;
    if first.starts_with('-') {
        return None;
    }
    const SUBCOMMANDS: &[&str] = &[
        "add",
        "remove",
        "rm",
        "edit",
        "add-path",
        "edit-path",
        "remove-path",
        "rm-path",
        "set-default",
        "list",
        "ls",
        "completions",
        "help",
    ];
    if SUBCOMMANDS.contains(&first.as_str()) {
        return None;
    }
    Some(first.clone())
}

fn main_arg_position() -> Option<usize> {
    let words = subcommand_words();
    if words.len() < 2 {
        return None;
    }
    Some(words.len() - 2)
}

fn resolve_completion_target(
    partial: &str,
    home: Option<&std::path::Path>,
) -> Option<(std::path::PathBuf, String, String)> {
    if let Some(rest) = partial.strip_prefix("~/") {
        let home = home?;
        let home_str = home.to_string_lossy();
        let home_prefix = home_str.trim_end_matches('/');
        let (read_dir, display_prefix, name_prefix) = match rest.rsplit_once('/') {
            Some((sub, p)) => (
                home.join(sub),
                format!("{home_prefix}/{sub}/"),
                p.to_string(),
            ),
            None => (
                home.to_path_buf(),
                format!("{home_prefix}/"),
                rest.to_string(),
            ),
        };
        Some((read_dir, display_prefix, name_prefix))
    } else if let Some(rest) = partial.strip_prefix("./") {
        let (read_dir, display_prefix, name_prefix) = match rest.rsplit_once('/') {
            Some((sub, p)) => (
                std::path::PathBuf::from(".").join(sub),
                format!("./{sub}/"),
                p.to_string(),
            ),
            None => (
                std::path::PathBuf::from("."),
                "./".to_string(),
                rest.to_string(),
            ),
        };
        Some((read_dir, display_prefix, name_prefix))
    } else {
        let (read_dir, display_prefix, name_prefix) = match partial.rsplit_once('/') {
            Some(("", p)) => (
                std::path::PathBuf::from("/"),
                "/".to_string(),
                p.to_string(),
            ),
            Some((d, p)) => (std::path::PathBuf::from(d), format!("{d}/"), p.to_string()),
            None => (
                std::path::PathBuf::from("."),
                String::new(),
                partial.to_string(),
            ),
        };
        Some((read_dir, display_prefix, name_prefix))
    }
}

fn local_file_candidates(partial: &str) -> Vec<CompletionCandidate> {
    local_file_candidates_with_home(partial, dirs::home_dir().as_deref())
}

fn local_file_candidates_with_home(
    partial: &str,
    home: Option<&std::path::Path>,
) -> Vec<CompletionCandidate> {
    let Some((read_dir_path, display_prefix, name_prefix)) =
        resolve_completion_target(partial, home)
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&read_dir_path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !name.starts_with(&name_prefix) {
            continue;
        }
        if name.starts_with('.') && !name_prefix.starts_with('.') {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let full = format!("{display_prefix}{name}");
        let display = if is_dir { format!("{full}/") } else { full };
        out.push(CompletionCandidate::new(display));
    }
    out
}

fn complete_main_positional(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let partial = current.to_str().unwrap_or("");
    let mut candidates = Vec::new();

    if main_arg_position() == Some(0)
        && let Some(server) = extract_main_server_arg()
    {
        let servers = load_servers();
        if let Some(srv) = servers.get(&server) {
            for (k, v) in &srv.paths {
                if !partial.is_empty() && !fuzzy_match(partial, k) {
                    continue;
                }
                let help = if k == &srv.default {
                    format!("{v} (default)")
                } else {
                    v.clone()
                };
                candidates.push(
                    CompletionCandidate::new(k).help(Some(clap::builder::StyledStr::from(help))),
                );
            }
        }
    }

    candidates.extend(local_file_candidates(partial));
    candidates
}

fn complete_remote_path(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(host) = extract_host_for_path_completion() else {
        return Vec::new();
    };
    let raw_current = current.to_str().unwrap_or("");
    let current = raw_current.replace("\\~", "~");
    let current = current.as_str();
    let ls_dir = if current.is_empty() {
        "~".to_string()
    } else if current.ends_with('/') {
        current.trim_end_matches('/').to_string()
    } else {
        current
            .rsplit_once('/')
            .map(|(p, _)| p.to_string())
            .unwrap_or("~".to_string())
    };

    let cache = cache_dir();
    let key = cache_key(&host, &ls_dir);
    let cache_file = cache.join(&key);

    spawn_remote_ls(&host, &ls_dir);

    let cached = match std::fs::read_to_string(&cache_file) {
        Ok(c) if !c.is_empty() => c,
        _ => {
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(200));
                if let Ok(c) = std::fs::read_to_string(&cache_file)
                    && !c.is_empty()
                {
                    break;
                }
            }
            match std::fs::read_to_string(&cache_file) {
                Ok(c) if !c.is_empty() => c,
                _ => return Vec::new(),
            }
        }
    };

    let mut lines = cached.lines().filter(|l| !l.is_empty());
    let resolved_dir = match lines.next() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let prefix = format!("{resolved_dir}/");

    let partial = if current.is_empty() || current.ends_with('/') {
        ""
    } else {
        current
            .rsplit_once('/')
            .map(|(_, name)| name)
            .unwrap_or(current)
    };

    lines
        .filter_map(|line| {
            let is_dir = line.ends_with('/');
            let entry = if is_dir {
                line.trim_end_matches('/')
            } else {
                line
            };
            if !partial.is_empty() && !entry.starts_with(partial) {
                return None;
            }
            let full = format!("{prefix}{entry}");
            let mut candidate = CompletionCandidate::new(&full);
            if is_dir {
                candidate = candidate.help(Some(clap::builder::StyledStr::from("dir")));
            }
            Some(candidate)
        })
        .collect()
}

#[derive(Parser)]
#[command(
    name = "snd",
    about = "Quick scp to configured servers",
    subcommand_negates_reqs = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Cmd>,

    #[arg(add = ArgValueCompleter::new(complete_server_alias))]
    pub server: Option<String>,

    #[arg(trailing_var_arg = true, add = ArgValueCompleter::new(complete_main_positional))]
    pub args: Vec<String>,
}

#[derive(Subcommand)]
pub enum Cmd {
    Add {
        #[arg(value_name = "ALIAS")]
        alias: String,
        #[arg(value_name = "HOST", add = ArgValueCompleter::new(complete_ssh_target))]
        host: String,
        #[arg(value_name = "/REMOTE/PATH", add = ArgValueCompleter::new(complete_remote_path))]
        path: Option<String>,
    },
    #[command(alias = "rm")]
    Remove {
        #[arg(add = ArgValueCompleter::new(complete_server_alias))]
        alias: String,
    },
    Edit {
        #[arg(add = ArgValueCompleter::new(complete_server_alias))]
        alias: String,
        #[arg(value_name = "HOST", add = ArgValueCompleter::new(complete_ssh_target))]
        host: String,
    },
    #[command(name = "add-path")]
    AddPath {
        #[arg(add = ArgValueCompleter::new(complete_server_alias))]
        server: String,
        #[arg(value_name = "PATH_ALIAS")]
        path_alias: String,
        #[arg(value_name = "/REMOTE/PATH", add = ArgValueCompleter::new(complete_remote_path))]
        path: String,
    },
    #[command(name = "edit-path")]
    EditPath {
        #[arg(add = ArgValueCompleter::new(complete_server_alias))]
        server: String,
        #[arg(value_name = "PATH_ALIAS", add = ArgValueCompleter::new(complete_path_alias))]
        path_alias: String,
        #[arg(value_name = "/REMOTE/PATH", add = ArgValueCompleter::new(complete_remote_path))]
        path: String,
    },
    #[command(name = "remove-path", alias = "rm-path")]
    RemovePath {
        #[arg(add = ArgValueCompleter::new(complete_server_alias))]
        server: String,
        #[arg(value_name = "PATH_ALIAS", add = ArgValueCompleter::new(complete_path_alias))]
        path_alias: String,
    },
    #[command(name = "set-default")]
    SetDefault {
        #[arg(add = ArgValueCompleter::new(complete_server_alias))]
        server: String,
        #[arg(value_name = "PATH_ALIAS", add = ArgValueCompleter::new(complete_path_alias))]
        path_alias: String,
    },
    #[command(alias = "ls")]
    List,
    Completions {
        shell: Shell,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_match_empty_pattern_matches_anything() {
        assert!(fuzzy_match("", "foo"));
        assert!(fuzzy_match("", ""));
    }

    #[test]
    fn fuzzy_match_substring() {
        assert!(fuzzy_match("abc", "abc"));
        assert!(fuzzy_match("abc", "aXbXc"));
        assert!(fuzzy_match("dpl", "deploy"));
    }

    #[test]
    fn fuzzy_match_case_insensitive() {
        assert!(fuzzy_match("abc", "ABC"));
        assert!(fuzzy_match("DPL", "deploy"));
    }

    #[test]
    fn fuzzy_match_order_matters() {
        assert!(!fuzzy_match("cba", "abc"));
    }

    #[test]
    fn fuzzy_match_pattern_longer_than_target() {
        assert!(!fuzzy_match("abcd", "abc"));
    }

    #[test]
    fn cache_key_sanitizes_host_separators() {
        assert_eq!(
            cache_key("user@host:22", "/var/log"),
            "user_host_22-_var_log"
        );
    }

    #[test]
    fn cache_key_handles_tilde_home() {
        assert_eq!(cache_key("host", "~"), "host-~");
    }

    #[test]
    fn local_file_candidates_lists_dir_and_marks_subdirs() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("snd-lfc-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("alpha.txt"), "").unwrap();
        std::fs::write(dir.join("beta.txt"), "").unwrap();
        std::fs::create_dir(dir.join("subdir")).unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();

        let partial = format!("{}/", dir.display());
        let cands = local_file_candidates(&partial);
        let values: Vec<String> = cands
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect();

        assert!(values.iter().any(|v| v.ends_with("/alpha.txt")));
        assert!(values.iter().any(|v| v.ends_with("/beta.txt")));
        assert!(
            values.iter().any(|v| v.ends_with("/subdir/")),
            "dirs should have trailing slash: {values:?}"
        );
        assert!(
            !values.iter().any(|v| v.ends_with("/.hidden")),
            "hidden files excluded when prefix doesn't start with dot"
        );

        let prefix_cands = local_file_candidates(&format!("{}/al", dir.display()));
        let prefix_values: Vec<String> = prefix_cands
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect();
        assert_eq!(prefix_values.len(), 1);
        assert!(prefix_values[0].ends_with("/alpha.txt"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_completion_target_tilde_expands_to_home_in_prefix() {
        let home = std::path::PathBuf::from("/fake/home");
        let (dir, prefix, name) = resolve_completion_target("~/", Some(&home)).unwrap();
        assert_eq!(dir, home);
        assert_eq!(prefix, "/fake/home/");
        assert_eq!(name, "");

        let (dir, prefix, name) = resolve_completion_target("~/Doc", Some(&home)).unwrap();
        assert_eq!(dir, home);
        assert_eq!(prefix, "/fake/home/");
        assert_eq!(name, "Doc");

        let (dir, prefix, name) = resolve_completion_target("~/sub/foo", Some(&home)).unwrap();
        assert_eq!(dir, home.join("sub"));
        assert_eq!(prefix, "/fake/home/sub/");
        assert_eq!(name, "foo");
    }

    #[test]
    fn resolve_completion_target_tilde_without_home_returns_none() {
        assert!(resolve_completion_target("~/foo", None).is_none());
    }

    #[test]
    fn resolve_completion_target_dot_slash_preserves_prefix() {
        let (dir, prefix, name) = resolve_completion_target("./", None).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("."));
        assert_eq!(prefix, "./");
        assert_eq!(name, "");

        let (dir, prefix, name) = resolve_completion_target("./foo", None).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("."));
        assert_eq!(prefix, "./");
        assert_eq!(name, "foo");

        let (dir, prefix, name) = resolve_completion_target("./sub/foo", None).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("./sub"));
        assert_eq!(prefix, "./sub/");
        assert_eq!(name, "foo");
    }

    #[test]
    fn resolve_completion_target_absolute_and_bare() {
        let (dir, prefix, name) = resolve_completion_target("/etc/pass", None).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("/etc"));
        assert_eq!(prefix, "/etc/");
        assert_eq!(name, "pass");

        let (dir, prefix, name) = resolve_completion_target("/foo", None).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("/"));
        assert_eq!(prefix, "/");
        assert_eq!(name, "foo");

        let (dir, prefix, name) = resolve_completion_target("foo", None).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("."));
        assert_eq!(prefix, "");
        assert_eq!(name, "foo");
    }

    #[test]
    fn local_file_candidates_with_home_expands_tilde() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let home = std::env::temp_dir().join(format!("snd-home-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("notes.txt"), "").unwrap();
        std::fs::create_dir(home.join("projects")).unwrap();

        let home_str = home.to_string_lossy().into_owned();
        let cands = local_file_candidates_with_home("~/", Some(&home));
        let values: Vec<String> = cands
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect();
        let expected_notes = format!("{home_str}/notes.txt");
        let expected_projects = format!("{home_str}/projects/");
        assert!(
            values.iter().any(|v| v == &expected_notes),
            "values: {values:?}"
        );
        assert!(
            values.iter().any(|v| v == &expected_projects),
            "values: {values:?}"
        );

        let filtered = local_file_candidates_with_home("~/not", Some(&home));
        let filtered_values: Vec<String> = filtered
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect();
        assert_eq!(filtered_values, vec![expected_notes]);

        std::fs::remove_dir_all(&home).ok();
    }
}
