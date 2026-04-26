use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub default: String,
    pub paths: BTreeMap<String, String>,
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

pub type Servers = BTreeMap<String, Server>;

pub fn config_path() -> PathBuf {
    config_dir().join("servers.toml")
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("snd")
}

fn legacy_path() -> PathBuf {
    config_dir().join("servers.conf")
}

pub fn load_servers() -> Servers {
    let path = config_path();
    if let Ok(content) = fs::read_to_string(&path)
        && let Ok(servers) = toml::from_str::<Servers>(&content)
    {
        return servers;
    }

    let legacy = legacy_path();
    if let Ok(content) = fs::read_to_string(&legacy) {
        let servers = parse_legacy(&content);
        if !servers.is_empty() {
            let _ = save_servers(&servers);
        }
        return servers;
    }

    Servers::new()
}

fn parse_legacy(content: &str) -> Servers {
    let mut servers = Servers::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((alias, target)) = line.split_once('=') else {
            continue;
        };
        let (host, path) = match target.split_once(':') {
            Some((h, p)) => (h.to_string(), p.to_string()),
            None => (target.to_string(), "~".to_string()),
        };
        let mut paths = BTreeMap::new();
        paths.insert("default".to_string(), path);
        servers.insert(
            alias.to_string(),
            Server {
                host,
                default: "default".to_string(),
                paths,
            },
        );
    }
    servers
}

pub fn save_servers(servers: &Servers) -> io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(servers).map_err(io::Error::other)?;
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
    fn legacy_single_entry() {
        let servers = parse_legacy("web=user@host:/var/www\n");
        assert_eq!(servers.len(), 1);
        let web = &servers["web"];
        assert_eq!(web.host, "user@host");
        assert_eq!(web.default, "default");
        assert_eq!(web.paths["default"], "/var/www");
    }

    #[test]
    fn legacy_multiple_entries() {
        let servers = parse_legacy("web=u@h:/var/www\ndb=dbhost:/opt/db\n");
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["web"].host, "u@h");
        assert_eq!(servers["db"].host, "dbhost");
    }

    #[test]
    fn legacy_ignores_comments_and_blank_lines() {
        let input = "# snd server config\n\n# Format: alias=host:path\nweb=h:/p\n\n";
        let servers = parse_legacy(input);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["web"].paths["default"], "/p");
    }

    #[test]
    fn legacy_defaults_path_when_missing() {
        let servers = parse_legacy("alias=host\n");
        assert_eq!(servers["alias"].host, "host");
        assert_eq!(servers["alias"].paths["default"], "~");
    }

    #[test]
    fn legacy_splits_on_first_colon() {
        let servers = parse_legacy("s=host:/path:with:colons\n");
        assert_eq!(servers["s"].host, "host");
        assert_eq!(servers["s"].paths["default"], "/path:with:colons");
    }

    #[test]
    fn legacy_skips_malformed_lines() {
        let servers = parse_legacy("no_equals_sign\nok=h:/p\n");
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("ok"));
    }

    #[test]
    fn toml_roundtrip() {
        let mut servers = Servers::new();
        servers.insert(
            "web".to_string(),
            make_server("u@h", "main", &[("main", "/var/www"), ("logs", "/var/log")]),
        );
        servers.insert(
            "db".to_string(),
            make_server("db.h", "root", &[("root", "/")]),
        );

        let serialized = toml::to_string_pretty(&servers).unwrap();
        let deserialized: Servers = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized["web"].host, "u@h");
        assert_eq!(deserialized["web"].default, "main");
        assert_eq!(deserialized["web"].paths.len(), 2);
        assert_eq!(deserialized["web"].paths["logs"], "/var/log");
        assert_eq!(deserialized["db"].paths["root"], "/");
    }
}
