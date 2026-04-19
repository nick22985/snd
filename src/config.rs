use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("snd")
        .join("servers.conf")
}

pub fn load_servers() -> BTreeMap<String, String> {
    let path = config_path();
    let mut servers = BTreeMap::new();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return servers,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((alias, target)) = line.split_once('=') {
            servers.insert(alias.to_string(), target.to_string());
        }
    }
    servers
}

pub fn save_servers(servers: &BTreeMap<String, String>) -> io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(&path)?;
    writeln!(f, "# snd server config")?;
    writeln!(f, "# Format: alias=host:path")?;
    for (alias, target) in servers {
        writeln!(f, "{alias}={target}")?;
    }
    Ok(())
}
