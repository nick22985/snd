use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshResolved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub default: String,
    pub paths: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<SshResolved>,
}

impl Server {
    pub fn path_for(&self, path_alias: &str) -> Option<&String> {
        self.paths.get(path_alias)
    }

    pub fn default_path(&self) -> Option<&String> {
        self.paths.get(&self.default)
    }

    pub fn target_for(&self, path_alias: &str) -> Option<String> {
        self.path_for(path_alias)
            .map(|p| format!("{}:{}", self.host, p))
    }

    pub fn default_target(&self) -> Option<String> {
        self.default_path().map(|p| format!("{}:{}", self.host, p))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Group {
    #[serde(default)]
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub servers: BTreeMap<String, Server>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, Group>,
}

pub type Servers = BTreeMap<String, Server>;

pub struct GroupTarget<'a> {
    pub server: &'a str,
    pub path_alias: Option<&'a str>,
}

pub fn parse_group_target(s: &str) -> GroupTarget<'_> {
    match s.split_once(':') {
        Some((server, path)) => GroupTarget {
            server,
            path_alias: Some(path),
        },
        None => GroupTarget {
            server: s,
            path_alias: None,
        },
    }
}

pub fn canonicalize_group_target(cfg: &Config, token: &str) -> Result<String, String> {
    let gt = parse_group_target(token);

    if let Some(srv) = cfg.servers.get(gt.server) {
        if let Some(p) = gt.path_alias
            && !srv.paths.contains_key(p)
        {
            return Err(format!(
                "target '{token}': path '{p}' not found on server '{}'.",
                gt.server
            ));
        }
        return Ok(token.to_string());
    }

    if gt.path_alias.is_some() {
        return Err(format!(
            "target '{token}': server '{}' not found.",
            gt.server
        ));
    }

    let matches: Vec<&String> = cfg
        .servers
        .iter()
        .filter(|(_, s)| s.paths.contains_key(token))
        .map(|(alias, _)| alias)
        .collect();
    match matches.as_slice() {
        [] => Err(format!(
            "target '{token}' not found (no server or path alias by that name)."
        )),
        [server] => Ok(format!("{server}:{token}")),
        many => {
            let options = many
                .iter()
                .map(|s| format!("'{s}:{token}'"))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "path alias '{token}' is ambiguous — matches multiple servers. Use one of: {options}"
            ))
        }
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("servers.toml")
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("snd")
}

pub fn load_config_strict() -> Result<Config, String> {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(content) => toml::from_str::<Config>(&content)
            .map_err(|e| format!("failed to parse {}: {e}", path.display())),
        Err(_) => Ok(Config::default()),
    }
}

pub fn load_config() -> Config {
    load_config_strict().unwrap_or_default()
}

pub fn load_servers() -> Servers {
    load_config().servers
}

pub fn save_config(cfg: &Config) -> io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(cfg).map_err(io::Error::other)?;
    fs::write(&path, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server(host: &str, default: &str, paths: &[(&str, &str)]) -> Server {
        let mut p = BTreeMap::new();
        for (k, v) in paths {
            p.insert(k.to_string(), v.to_string());
        }
        Server {
            host: host.to_string(),
            default: default.to_string(),
            paths: p,
            resolved: None,
        }
    }

    #[test]
    fn server_path_lookups() {
        let s = make_server("h", "main", &[("main", "/x"), ("logs", "/var/log")]);
        assert_eq!(s.path_for("main"), Some(&"/x".to_string()));
        assert_eq!(s.path_for("logs"), Some(&"/var/log".to_string()));
        assert_eq!(s.path_for("missing"), None);
    }

    #[test]
    fn server_targets() {
        let s = make_server("u@host", "main", &[("main", "/x"), ("logs", "/var/log")]);
        assert_eq!(s.default_target(), Some("u@host:/x".to_string()));
        assert_eq!(s.target_for("logs"), Some("u@host:/var/log".to_string()));
        assert_eq!(s.target_for("missing"), None);
    }

    #[test]
    fn server_default_missing_is_none() {
        let s = make_server("h", "gone", &[("main", "/x")]);
        assert_eq!(s.default_path(), None);
        assert_eq!(s.default_target(), None);
    }

    #[test]
    fn config_roundtrip_with_groups() {
        let mut servers = Servers::new();
        servers.insert(
            "web".to_string(),
            make_server("u@h", "main", &[("main", "/var/www"), ("logs", "/var/log")]),
        );
        servers.insert(
            "db".to_string(),
            make_server("db.h", "root", &[("root", "/")]),
        );
        let mut groups = BTreeMap::new();
        groups.insert(
            "prod".to_string(),
            Group {
                targets: vec!["web".to_string(), "db:root".to_string()],
            },
        );
        let cfg = Config { servers, groups };

        let serialized = toml::to_string_pretty(&cfg).unwrap();
        assert!(serialized.contains("[servers.web]"));
        assert!(serialized.contains("[groups.prod]"));

        let de: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(de.servers.len(), 2);
        assert_eq!(de.servers["web"].paths["logs"], "/var/log");
        assert_eq!(de.groups["prod"].targets, vec!["web", "db:root"]);
    }

    #[test]
    fn parse_new_format_with_groups() {
        let new = r#"
[servers.web]
host = "u@h"
default = "default"
[servers.web.paths]
default = "/var/www"

[groups.prod]
targets = ["web", "db:root"]
"#;
        let cfg: Config = toml::from_str(new).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers["web"].host, "u@h");
        assert_eq!(cfg.groups["prod"].targets, vec!["web", "db:root"]);
    }

    #[test]
    fn parse_group_target_splits_on_colon() {
        let t = parse_group_target("web");
        assert_eq!(t.server, "web");
        assert!(t.path_alias.is_none());

        let t = parse_group_target("web:logs");
        assert_eq!(t.server, "web");
        assert_eq!(t.path_alias, Some("logs"));
    }

    fn cfg_with_servers(servers: &[(&str, Server)]) -> Config {
        let mut s = Servers::new();
        for (name, srv) in servers {
            s.insert(name.to_string(), srv.clone());
        }
        Config {
            servers: s,
            groups: BTreeMap::new(),
        }
    }

    #[test]
    fn canonicalize_bare_server_kept_verbatim() {
        let cfg = cfg_with_servers(&[("web", make_server("u@h", "default", &[("default", "/x")]))]);
        assert_eq!(canonicalize_group_target(&cfg, "web").unwrap(), "web");
    }

    #[test]
    fn canonicalize_bare_unique_path_alias_gets_qualified() {
        let cfg = cfg_with_servers(&[(
            "box1",
            make_server(
                "u@h",
                "default",
                &[("default", "/x"), ("spawn", "/srv/spawn")],
            ),
        )]);
        assert_eq!(
            canonicalize_group_target(&cfg, "spawn").unwrap(),
            "box1:spawn"
        );
    }

    #[test]
    fn canonicalize_server_name_wins_over_path_alias() {
        let cfg = cfg_with_servers(&[
            (
                "spawn",
                make_server("u@h1", "default", &[("default", "/a")]),
            ),
            (
                "box1",
                make_server(
                    "u@h2",
                    "default",
                    &[("default", "/b"), ("spawn", "/srv/spawn")],
                ),
            ),
        ]);
        assert_eq!(canonicalize_group_target(&cfg, "spawn").unwrap(), "spawn");
    }

    #[test]
    fn canonicalize_ambiguous_path_alias_errors() {
        let cfg = cfg_with_servers(&[
            (
                "a",
                make_server("u@h1", "default", &[("default", "/a"), ("shared", "/a/s")]),
            ),
            (
                "b",
                make_server("u@h2", "default", &[("default", "/b"), ("shared", "/b/s")]),
            ),
        ]);
        let err = canonicalize_group_target(&cfg, "shared").unwrap_err();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(
            err.contains("'a:shared'") && err.contains("'b:shared'"),
            "{err}"
        );
    }

    #[test]
    fn canonicalize_explicit_qualified_validates_path() {
        let cfg = cfg_with_servers(&[("web", make_server("u@h", "default", &[("default", "/x")]))]);
        assert_eq!(
            canonicalize_group_target(&cfg, "web:default").unwrap(),
            "web:default"
        );
        let err = canonicalize_group_target(&cfg, "web:nope").unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn canonicalize_explicit_qualified_unknown_server_errors() {
        let cfg = cfg_with_servers(&[("web", make_server("u@h", "default", &[("default", "/x")]))]);
        let err = canonicalize_group_target(&cfg, "ghost:default").unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn canonicalize_unknown_bare_token_errors() {
        let cfg = cfg_with_servers(&[("web", make_server("u@h", "default", &[("default", "/x")]))]);
        let err = canonicalize_group_target(&cfg, "nope").unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn empty_groups_omitted_from_serialized_config() {
        let cfg = Config {
            servers: Servers::new(),
            groups: BTreeMap::new(),
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(!s.contains("[groups"), "expected no groups header: {s}");
    }
}
