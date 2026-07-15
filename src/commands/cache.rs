use serde::Serialize;
use std::io;

use snd::cli::completion_cache_dir;
use snd::report::print_json;

#[derive(Serialize)]
struct CacheEntry {
    path: String,
    bytes: u64,
    age_seconds: u64,
}

pub fn run(action: &str, older_than: Option<u64>, json: bool) -> i32 {
    let directory = completion_cache_dir();
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if json {
                print_json("cache", true, &Vec::<CacheEntry>::new());
            } else {
                println!("Completion cache is empty ({}).", directory.display());
            }
            return 0;
        }
        Err(e) => {
            eprintln!("Failed to read {}: {e}", directory.display());
            return 1;
        }
    };
    let cutoff = older_than.map(|days| days.saturating_mul(24 * 60 * 60));
    let mut records = Vec::new();
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        if cutoff.is_some_and(|cutoff| age < cutoff) {
            continue;
        }
        records.push(CacheEntry {
            path: entry.path().display().to_string(),
            bytes: metadata.len(),
            age_seconds: age,
        });
        paths.push(entry.path());
    }
    if action == "clear" {
        let mut failed = 0;
        for path in &paths {
            if let Err(e) = std::fs::remove_file(path) {
                failed += 1;
                eprintln!("Failed to remove {}: {e}", path.display());
            }
        }
        if json {
            print_json("cache-clear", failed == 0, &records);
        } else {
            println!(
                "Removed {} completion cache file(s).",
                records.len() - failed
            );
        }
        return i32::from(failed > 0);
    }
    if json {
        print_json("cache", true, &records);
    } else if records.is_empty() {
        println!("Completion cache is empty ({}).", directory.display());
    } else {
        println!("Completion cache: {}", directory.display());
        for record in &records {
            println!(
                "  {:>8} bytes  {:>8}s  {}",
                record.bytes, record.age_seconds, record.path
            );
        }
    }
    0
}
