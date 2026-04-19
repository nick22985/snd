use std::process::Command;
use std::sync::mpsc;
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
        .filter(|(alias, target)| {
            current.is_empty() || fuzzy_match(current, alias) || fuzzy_match(current, target)
        })
        .map(|(alias, target)| {
            CompletionCandidate::new(&alias).help(Some(clap::builder::StyledStr::from(target)))
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

fn extract_host_from_args() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let words: Vec<&str> = args
        .iter()
        .skip_while(|a| *a != "--")
        .skip(1)
        .map(|s| s.as_str())
        .collect();
    for (i, w) in words.iter().enumerate() {
        if (*w == "add" || *w == "edit") && i + 2 < words.len() {
            return Some(words[i + 2].to_string());
        }
    }
    None
}

fn complete_remote_path(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(host) = extract_host_from_args() else {
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

    let (tx, rx) = mpsc::channel();
    let ssh_host = host.clone();
    let remote_dir = ls_dir.clone();
    std::thread::spawn(move || {
        let result = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=2",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPath=~/.ssh/snd-%r@%h:%p",
                "-o",
                "ControlPersist=60",
                &ssh_host,
                &format!("cd {remote_dir} && pwd && ls -1p"),
            ])
            .output();
        let _ = tx.send(result);
    });

    let output = match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(out)) if out.status.success() => out,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines().filter(|l| !l.is_empty());
    let _resolved_dir = match lines.next() {
        Some(d) => d,
        None => return Vec::new(),
    };

    let user_prefix = if current.is_empty() || current.ends_with('/') {
        current.to_string()
    } else {
        match current.rsplit_once('/') {
            Some((p, _)) => format!("{p}/"),
            None => String::new(),
        }
    };
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
            let full = format!("{user_prefix}{entry}");
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

    #[arg(trailing_var_arg = true, value_hint = clap::ValueHint::FilePath)]
    pub files: Vec<String>,
}

#[derive(Subcommand)]
pub enum Cmd {
    Add {
        #[arg(value_name = "ALIAS")]
        alias: String,
        #[arg(value_name = "HOST", add = ArgValueCompleter::new(complete_ssh_target))]
        host: String,
        #[arg(value_name = "/REMOTE/PATH", default_value = "~", add = ArgValueCompleter::new(complete_remote_path))]
        path: String,
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
        #[arg(value_name = "/REMOTE/PATH", default_value = "~", add = ArgValueCompleter::new(complete_remote_path))]
        path: String,
    },
    #[command(alias = "ls")]
    List,
    Completions {
        shell: Shell,
    },
}
