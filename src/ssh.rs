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
    let expanded = if let Some(rest) = pattern.strip_prefix("~/") {
        home.join(rest).to_string_lossy().into_owned()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_file(content: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("snd-ssh-test-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn display_target_user_and_hostname() {
        let h = SshHost {
            alias: "a".into(),
            hostname: Some("h.example".into()),
            user: Some("bob".into()),
        };
        assert_eq!(h.display_target(), "bob@h.example");
    }

    #[test]
    fn display_target_user_only_uses_alias() {
        let h = SshHost {
            alias: "a".into(),
            hostname: None,
            user: Some("bob".into()),
        };
        assert_eq!(h.display_target(), "bob@a");
    }

    #[test]
    fn display_target_hostname_only() {
        let h = SshHost {
            alias: "a".into(),
            hostname: Some("h".into()),
            user: None,
        };
        assert_eq!(h.display_target(), "h");
    }

    #[test]
    fn display_target_alias_only() {
        let h = SshHost {
            alias: "a".into(),
            hostname: None,
            user: None,
        };
        assert_eq!(h.display_target(), "a");
    }

    #[test]
    fn parse_simple_host() {
        let path = tmp_file("Host web\n    Hostname web.example.com\n    User deploy\n");
        let mut hosts = Vec::new();
        parse_ssh_config(&path, &std::env::temp_dir(), &mut hosts);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "web");
        assert_eq!(hosts[0].hostname.as_deref(), Some("web.example.com"));
        assert_eq!(hosts[0].user.as_deref(), Some("deploy"));
    }

    #[test]
    fn parse_skips_wildcard_hosts() {
        let path = tmp_file("Host *\n    User default_user\n\nHost real\n    Hostname real.host\n");
        let mut hosts = Vec::new();
        parse_ssh_config(&path, &std::env::temp_dir(), &mut hosts);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "real");
    }

    #[test]
    fn parse_multiple_hosts() {
        let path = tmp_file("Host a\n    Hostname ah\n\nHost b\n    Hostname bh\n    User bu\n");
        let mut hosts = Vec::new();
        parse_ssh_config(&path, &std::env::temp_dir(), &mut hosts);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].alias, "a");
        assert_eq!(hosts[1].alias, "b");
        assert_eq!(hosts[1].user.as_deref(), Some("bu"));
    }

    #[test]
    fn parse_handles_equals_syntax() {
        let path = tmp_file("Host=web\nHostname=h\nUser=u\n");
        let mut hosts = Vec::new();
        parse_ssh_config(&path, &std::env::temp_dir(), &mut hosts);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].hostname.as_deref(), Some("h"));
        assert_eq!(hosts[0].user.as_deref(), Some("u"));
    }

    #[test]
    fn parse_ignores_comments() {
        let path = tmp_file("# comment\nHost a\n    # another\n    Hostname h\n");
        let mut hosts = Vec::new();
        parse_ssh_config(&path, &std::env::temp_dir(), &mut hosts);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].hostname.as_deref(), Some("h"));
    }

    #[test]
    fn parse_missing_file_is_noop() {
        let mut hosts = Vec::new();
        parse_ssh_config(
            &PathBuf::from("/nonexistent/snd-ssh-test-missing"),
            &std::env::temp_dir(),
            &mut hosts,
        );
        assert!(hosts.is_empty());
    }

    fn make_home() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let home = std::env::temp_dir().join(format!("snd-ssh-home-{}-{n}", std::process::id()));
        fs::create_dir_all(home.join(".ssh")).unwrap();
        home
    }

    #[test]
    fn resolve_include_expands_tilde() {
        let home = make_home();
        let target = home.join(".ssh/foo.conf");
        fs::write(&target, "").unwrap();
        let resolved = resolve_include("~/.ssh/foo.conf", &home);
        assert_eq!(resolved, vec![target]);
    }

    #[test]
    fn resolve_include_absolute_path() {
        let home = make_home();
        let target = home.join(".ssh/abs.conf");
        fs::write(&target, "").unwrap();
        let resolved = resolve_include(&target.to_string_lossy(), &home);
        assert_eq!(resolved, vec![target]);
    }

    #[test]
    fn resolve_include_relative_resolves_under_ssh_dir() {
        let home = make_home();
        let target = home.join(".ssh/rel.conf");
        fs::write(&target, "").unwrap();
        let resolved = resolve_include("rel.conf", &home);
        assert_eq!(resolved, vec![target]);
    }

    #[test]
    fn resolve_include_glob_expands() {
        let home = make_home();
        fs::create_dir_all(home.join(".ssh/conf.d")).unwrap();
        fs::write(home.join(".ssh/conf.d/a.conf"), "").unwrap();
        fs::write(home.join(".ssh/conf.d/b.conf"), "").unwrap();
        fs::write(home.join(".ssh/conf.d/skip.txt"), "").unwrap();
        let resolved = resolve_include("conf.d/*.conf", &home);
        assert_eq!(resolved.len(), 2);
        assert!(
            resolved
                .iter()
                .all(|p| p.extension().and_then(|e| e.to_str()) == Some("conf"))
        );
    }

    #[test]
    fn resolve_include_missing_pattern_returns_empty() {
        let home = make_home();
        let resolved = resolve_include("~/.ssh/nothing-here.conf", &home);
        assert!(resolved.is_empty());
    }

    #[test]
    fn parse_include_directive_inlines_hosts() {
        let home = make_home();
        fs::write(
            home.join(".ssh/extra.conf"),
            "Host included\n  Hostname ih\n  User iu\n",
        )
        .unwrap();

        let main = home.join(".ssh/main.conf");
        fs::write(
            &main,
            "Host before\n  Hostname bh\nInclude extra.conf\nHost after\n  Hostname ah\n",
        )
        .unwrap();

        let mut hosts = Vec::new();
        parse_ssh_config(&main, &home, &mut hosts);

        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, vec!["before", "included", "after"]);
        let included = hosts.iter().find(|h| h.alias == "included").unwrap();
        assert_eq!(included.hostname.as_deref(), Some("ih"));
        assert_eq!(included.user.as_deref(), Some("iu"));
    }
}
