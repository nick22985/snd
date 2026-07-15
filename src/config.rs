use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub version: u32,
    #[serde(default)]
    pub servers: BTreeMap<String, Server>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, Group>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_SCHEMA_VERSION,
            servers: BTreeMap::new(),
            groups: BTreeMap::new(),
        }
    }
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

pub fn project_config_path() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(".snd.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn project_config_path_for_write() -> PathBuf {
    project_config_path().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".snd.toml")
    })
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("snd")
}

pub fn load_config_path(path: &std::path::Path) -> Result<Config, String> {
    match fs::read_to_string(path) {
        Ok(content) => {
            let cfg = toml::from_str::<Config>(&content)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
            if cfg.version > CONFIG_SCHEMA_VERSION {
                return Err(format!(
                    "{} uses config schema {}, but this snd supports up to {}",
                    path.display(),
                    cfg.version,
                    CONFIG_SCHEMA_VERSION
                ));
            }
            Ok(cfg)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("failed to read {}: {e}", path.display())),
    }
}

pub fn load_config_strict() -> Result<Config, String> {
    load_config_path(&config_path())
}

pub fn load_project_config_strict() -> Result<(PathBuf, Config), String> {
    let path = project_config_path().ok_or_else(|| {
        "no project .snd.toml found; run 'snd init' before using --local".to_string()
    })?;
    let config = load_config_path(&path)?;
    Ok((path, config))
}

pub fn load_effective_config_strict() -> Result<Config, String> {
    let mut cfg = load_config_strict()?;
    if let Some(path) = project_config_path() {
        let project = load_config_path(&path)?;
        cfg.servers.extend(project.servers);
        cfg.groups.extend(project.groups);
    }
    Ok(cfg)
}

pub fn load_config() -> Config {
    load_effective_config_strict().unwrap_or_default()
}

pub fn load_servers() -> Servers {
    load_effective_config_strict().unwrap_or_default().servers
}

pub fn save_config(cfg: &Config) -> io::Result<()> {
    let path = config_path();
    save_config_path(cfg, &path, true)
}

pub fn save_config_path(cfg: &Config, path: &std::path::Path, private: bool) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut cfg = cfg.clone();
    cfg.version = CONFIG_SCHEMA_VERSION;
    let content = toml::to_string_pretty(&cfg).map_err(io::Error::other)?;
    let temp = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    let backup = path.with_extension("toml.bak");
    if path.exists() {
        fs::copy(path, &backup)?;
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&backup, fs::Permissions::from_mode(0o600))?;
        }
    }
    fs::write(&temp, content)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temp, path)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

pub fn validate_config(cfg: &Config) -> Vec<String> {
    let mut errors = Vec::new();
    if cfg.version != CONFIG_SCHEMA_VERSION {
        errors.push(format!(
            "schema version is {}, expected {}",
            cfg.version, CONFIG_SCHEMA_VERSION
        ));
    }
    for (alias, server) in &cfg.servers {
        if !valid_name(alias) {
            errors.push(format!("invalid server alias '{alias}'"));
        }
        if cfg.groups.contains_key(alias) {
            errors.push(format!("'{alias}' is both a server and a group"));
        }
        if server.host.trim().is_empty() {
            errors.push(format!("server '{alias}' has an empty host"));
        }
        if server.paths.is_empty() {
            errors.push(format!("server '{alias}' has no paths"));
        }
        if !server.paths.contains_key(&server.default) {
            errors.push(format!(
                "server '{alias}' default path alias '{}' does not exist",
                server.default
            ));
        }
        for (path_alias, path) in &server.paths {
            if !valid_name(path_alias) {
                errors.push(format!(
                    "server '{alias}' has invalid path alias '{path_alias}'"
                ));
            }
            if path.trim().is_empty() {
                errors.push(format!(
                    "server '{alias}' path alias '{path_alias}' is empty"
                ));
            }
        }
    }
    for (group, definition) in &cfg.groups {
        if !valid_name(group) {
            errors.push(format!("invalid group name '{group}'"));
        }
        if definition.targets.is_empty() {
            errors.push(format!("group '{group}' has no targets"));
        }
        for target in &definition.targets {
            if let Err(e) = canonicalize_group_target(cfg, target) {
                errors.push(format!("group '{group}': {e}"));
            }
        }
    }
    errors
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
        let cfg = Config {
            servers,
            groups,
            ..Config::default()
        };

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
            ..Config::default()
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
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(!s.contains("[groups"), "expected no groups header: {s}");
    }
}
