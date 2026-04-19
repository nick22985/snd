use std::fs;
use std::path::{Path, PathBuf};

pub struct SshHost {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
}

impl SshHost {
    pub fn display_target(&self) -> String {
        let actual_host = self.hostname.as_deref().unwrap_or(&self.alias);
        match &self.user {
            Some(u) => format!("{u}@{actual_host}"),
            None => actual_host.to_string(),
        }
    }
}

pub fn parse_ssh_hosts() -> Vec<SshHost> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let path = home.join(".ssh").join("config");
    let mut hosts = Vec::new();
    parse_ssh_config(&path, &home, &mut hosts);
    hosts
}

fn parse_ssh_config(path: &Path, home: &Path, hosts: &mut Vec<SshHost>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut current_host: Option<String> = None;
    let mut current_hostname: Option<String> = None;
    let mut current_user: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        match key.as_str() {
            "include" => {
                flush_host(
                    &mut current_host,
                    &mut current_hostname,
                    &mut current_user,
                    hosts,
                );
                for included in resolve_include(&value, home) {
                    parse_ssh_config(&included, home, hosts);
                }
            }
            "host" => {
                flush_host(
                    &mut current_host,
                    &mut current_hostname,
                    &mut current_user,
                    hosts,
                );
                if !value.contains('*') && !value.contains('?') {
                    current_host = Some(value);
                }
            }
            "hostname" => current_hostname = Some(value),
            "user" => current_user = Some(value),
            _ => {}
        }
    }

    flush_host(
        &mut current_host,
        &mut current_hostname,
        &mut current_user,
        hosts,
    );
}

fn flush_host(
    current_host: &mut Option<String>,
    current_hostname: &mut Option<String>,
    current_user: &mut Option<String>,
    hosts: &mut Vec<SshHost>,
) {
    if let Some(alias) = current_host.take() {
        hosts.push(SshHost {
            alias,
            hostname: current_hostname.take(),
            user: current_user.take(),
        });
    } else {
        *current_hostname = None;
        *current_user = None;
    }
}

fn resolve_include(pattern: &str, home: &Path) -> Vec<PathBuf> {
    let expanded = if pattern.starts_with("~/") {
        home.join(&pattern[2..]).to_string_lossy().into_owned()
    } else if pattern.starts_with('/') {
        pattern.to_string()
    } else {
        home.join(".ssh")
            .join(pattern)
            .to_string_lossy()
            .into_owned()
    };

    match glob::glob(&expanded) {
        Ok(paths) => paths.filter_map(|p| p.ok()).collect(),
        Err(_) => Vec::new(),
    }
}
