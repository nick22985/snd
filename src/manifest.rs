use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

fn default_version() -> u32 {
    MANIFEST_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default, rename = "deploy")]
    pub deployments: BTreeMap<String, Deployment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub target: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub atomic: bool,
    #[serde(default)]
    pub verify: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub preserve: bool,
    #[serde(default)]
    pub compress: bool,
    #[serde(default)]
    pub release: bool,
    #[serde(default)]
    pub release_name: Option<String>,
    #[serde(default = "default_keep")]
    pub keep: usize,
}

fn default_keep() -> usize {
    5
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read manifest {}: {e}", path.display()))?;
        let manifest: Self = toml::from_str(&content)
            .map_err(|e| format!("failed to parse manifest {}: {e}", path.display()))?;
        if manifest.version > MANIFEST_SCHEMA_VERSION {
            return Err(format!(
                "manifest schema {} is newer than the supported schema {}",
                manifest.version, MANIFEST_SCHEMA_VERSION
            ));
        }
        if manifest.deployments.is_empty() {
            return Err("manifest contains no [deploy.NAME] entries".to_string());
        }
        for (name, deployment) in &manifest.deployments {
            if deployment.target.trim().is_empty() {
                return Err(format!("deployment '{name}' has an empty target"));
            }
            if deployment.files.is_empty() {
                return Err(format!("deployment '{name}' has no files"));
            }
        }
        Ok(manifest)
    }

    pub fn selected(&self, name: Option<&str>) -> Result<Vec<(&str, &Deployment)>, String> {
        if let Some(name) = name {
            let (stored_name, deployment) = self
                .deployments
                .get_key_value(name)
                .ok_or_else(|| format!("deployment '{name}' not found in manifest"))?;
            return Ok(vec![(stored_name.as_str(), deployment)]);
        }
        Ok(self
            .deployments
            .iter()
            .map(|(name, deployment)| (name.as_str(), deployment))
            .collect())
    }

    pub fn resolved_files(manifest_path: &Path, deployment: &Deployment) -> Vec<String> {
        let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        deployment
            .files
            .iter()
            .map(|file| {
                let path = PathBuf::from(file);
                if path.is_absolute() {
                    path
                } else {
                    base.join(path)
                }
                .to_string_lossy()
                .into_owned()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_deployments() {
        let manifest: Manifest = toml::from_str(
            r#"
version = 1

[deploy.web]
target = "prod"
files = ["dist/app"]
atomic = true
verify = true
"#,
        )
        .unwrap();
        assert!(manifest.deployments["web"].atomic);
        assert!(manifest.deployments["web"].verify);
    }
}
