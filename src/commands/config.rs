use std::path::PathBuf;
use std::process::Command;

use snd::config::{
    Config, config_path, load_config_strict, load_project_config_strict, save_config,
    validate_config,
};

pub fn edit(local: bool) -> Result<PathBuf, String> {
    let path = if local {
        load_project_config_strict()?.0
    } else {
        let path = config_path();
        if !path.exists() {
            save_config(&Config::default())
                .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        }
        path
    };
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| "VISUAL/EDITOR is empty".to_string())?;
    let status = Command::new(program)
        .args(parts)
        .arg(&path)
        .status()
        .map_err(|error| format!("failed to start editor '{editor}': {error}"))?;
    if !status.success() {
        return Err(format!("editor exited with {}", status.code().unwrap_or(1)));
    }
    let config = if local {
        load_project_config_strict().map(|(_, config)| config)
    } else {
        load_config_strict()
    }
    .map_err(|error| format!("config error after editing: {error}"))?;
    let errors = validate_config(&config);
    if errors.is_empty() {
        Ok(path)
    } else {
        Err(format!(
            "configuration has {} problem(s):\n- {}",
            errors.len(),
            errors.join("\n- ")
        ))
    }
}
