use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct Envelope<'a, T: ?Sized> {
    schema_version: u32,
    command: &'a str,
    ok: bool,
    data: &'a T,
}

#[derive(Serialize)]
struct AuditRecord<'a, T: ?Sized> {
    schema_version: u32,
    timestamp_ms: u128,
    command: &'a str,
    ok: bool,
    data: &'a T,
}

pub fn print_json<T: Serialize + ?Sized>(command: &str, ok: bool, value: &T) {
    let envelope = Envelope {
        schema_version: JSON_SCHEMA_VERSION,
        command,
        ok,
        data: value,
    };
    match serde_json::to_string_pretty(&envelope) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("failed to serialize JSON: {e}"),
    }
}

pub fn append_audit<T: Serialize + ?Sized>(
    path: &Path,
    command: &str,
    ok: bool,
    value: &T,
) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create audit directory {}: {e}", parent.display()))?;
    }
    let record = AuditRecord {
        schema_version: JSON_SCHEMA_VERSION,
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        command,
        ok,
        data: value,
    };
    let line = serde_json::to_string(&record)
        .map_err(|e| format!("failed to serialize audit record: {e}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("failed to open audit log {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to secure audit log {}: {e}", path.display()))?;
    }
    writeln!(file, "{line}")
        .map_err(|e| format!("failed to write audit log {}: {e}", path.display()))
}
