use std::path::Path;

use serde_json::Value;
use snd::report::print_json;

pub fn run(
    path: &Path,
    limit: usize,
    command_filter: Option<&str>,
    failed_only: bool,
    json: bool,
) -> i32 {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            eprintln!("Failed to read audit log {}: {error}", path.display());
            return 1;
        }
    };
    let mut records = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                eprintln!(
                    "Invalid audit record at {}:{}: {error}",
                    path.display(),
                    index + 1
                );
                return 1;
            }
        };
        let matches_command = command_filter
            .is_none_or(|wanted| value.get("command").and_then(Value::as_str) == Some(wanted));
        let matches_status =
            !failed_only || value.get("ok").and_then(Value::as_bool) == Some(false);
        if matches_command && matches_status {
            records.push(value);
        }
    }
    records.reverse();
    if limit > 0 {
        records.truncate(limit);
    }

    if json {
        print_json("audit", true, &records);
        return 0;
    }
    if records.is_empty() {
        println!("No matching audit records in {}.", path.display());
        return 0;
    }
    println!("Audit log: {} (newest first)", path.display());
    for record in records {
        let timestamp = record
            .get("timestamp_ms")
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string());
        let command = record.get("command").and_then(Value::as_str).unwrap_or("?");
        let ok = record.get("ok").and_then(Value::as_bool).unwrap_or(false);
        println!(
            "\n{timestamp}  {:<8}  {}",
            command,
            if ok { "OK" } else { "FAILED" }
        );
        if let Some(results) = record.get("data").and_then(Value::as_array) {
            for result in results {
                let target = result.get("target").and_then(Value::as_str).unwrap_or("?");
                let action = result.get("action").and_then(Value::as_str).unwrap_or("?");
                let success = result.get("success").and_then(Value::as_bool).unwrap_or(ok);
                let message = result
                    .get("message")
                    .and_then(Value::as_str)
                    .map(|message| format!(" — {message}"))
                    .unwrap_or_default();
                println!(
                    "  {} {:<20} {:<14}{message}",
                    if success { "✓" } else { "✗" },
                    target,
                    action
                );
            }
        }
    }
    0
}
