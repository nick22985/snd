mod commands;

use clap::{CommandFactory, Parser};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use snd::backup::{
    SendHistoryEntry, SendRollback, acquire_send_lock, complete_send_backup, prepare_send_backup,
    release_send_lock, restore_send_backup, rollback_latest_send, rollback_named_sends,
    send_history,
};
use snd::cli::{Cli, Cmd};
use snd::config::{
    Config, Group, Server, SshResolved, canonicalize_group_target, config_path, load_config_strict,
    load_effective_config_strict, load_project_config_strict, parse_group_target,
    project_config_path, project_config_path_for_write, save_config, save_config_path,
    validate_config,
};
use snd::deploy::{
    acquire_deploy_lock, activate_release, complete_release, generated_release_name,
    prepare_release, prune_releases, release_deploy_lock, release_directory, release_state,
    remove_release, rollback_release, validate_release_name,
};
use snd::manifest::Manifest;
use snd::remote::{
    RemoteFileInfo, cat_remote, confirm, destination_basename, expand_remote_glob, find_remote,
    format_size, glob_label, grep_remote, has_glob, has_unescaped_glob, join_remote, ls_remote,
    rm_remote, scp_literal_remote_path, stat_remote, unescape_glob_literals,
};
use snd::report::{append_audit, print_json};
use snd::transfer::{
    OperationResult, TransferOptions, acquire_local_lock, acquire_remote_lock, local_sha256,
    release_local_lock, release_remote_lock, remote_rename, remote_sha256, remote_shell_path,
    retry_status, run_parallel, sftp_reget, sftp_reput, validate_download_resume,
    validate_upload_resume,
};

static TEMP_UPLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn load_or_exit() -> Config {
    load_effective_config_strict().unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    })
}

fn load_mutable_or_exit(local: bool) -> Config {
    let result = if local {
        load_project_config_strict().map(|(_, config)| config)
    } else {
        load_config_strict()
    };
    result.unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    })
}

fn save_mutable_or_exit(config: &Config, local: bool) {
    let result = if local {
        save_config_path(config, &project_config_path_for_write(), false)
    } else {
        save_config(config)
    };
    result.unwrap_or_else(|e| {
        eprintln!("Failed to write config: {e}");
        std::process::exit(1);
    });
}

fn transfer_options(cli: &Cli) -> TransferOptions {
    TransferOptions {
        dry_run: cli.dry_run,
        json: cli.json,
        jobs: cli.jobs.max(1),
        fail_fast: cli.fail_fast,
        retries: cli.retries,
        preserve: cli.preserve,
        compress: cli.compress,
        limit: cli.limit,
        identity: cli.identity.clone(),
        ssh_config: cli.ssh_config.clone(),
        atomic: cli.atomic,
        verify: cli.verify,
        resume: cli.resume,
        progress: cli.progress,
        audit_log: cli.audit_log.as_ref().map(PathBuf::from),
        backup: !cli.no_backup,
        backup_keep: cli.backup_keep,
    }
}

fn print_results(command: &str, results: &[OperationResult], options: &TransferOptions) -> i32 {
    let ok = results.iter().all(|result| result.success);
    if options.json {
        print_json(command, ok, results);
    } else if options.progress
        || matches!(command, "deploy" | "rollback")
        || results.len() > 1
        || results.iter().any(|r| !r.success)
    {
        println!();
        for result in results {
            let marker = if result.success { "✓" } else { "✗" };
            let retry = if result.attempts > 1 {
                format!(" ({} attempts)", result.attempts)
            } else {
                String::new()
            };
            let message = result
                .message
                .as_deref()
                .map(|m| format!(" — {m}"))
                .unwrap_or_default();
            let duration = if options.progress || result.duration_ms > 0 {
                format!(" in {} ms", result.duration_ms)
            } else {
                String::new()
            };
            let bytes = result
                .bytes
                .map(|bytes| format!(" ({})", format_size(bytes)))
                .unwrap_or_default();
            println!(
                "{marker} {:<24} {:<10}{retry}{bytes}{duration}{message}",
                result.target, result.action
            );
        }
        let succeeded = results.iter().filter(|r| r.success).count();
        let failed = results.len() - succeeded;
        println!("\n{succeeded} succeeded, {failed} failed");
    }
    if let Some(path) = &options.audit_log
        && let Err(e) = append_audit(path, command, ok, results)
    {
        eprintln!("Audit warning: {e}");
    }
    if ok { 0 } else { 1 }
}

fn confirm_for_output(prompt: &str, json: bool) -> bool {
    if !json {
        return confirm(prompt);
    }
    eprint!("{prompt} [y/N] ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim(), "y" | "Y" | "yes" | "YES" | "Yes")
}

fn validate_alias(label: &str, kind: &str) -> Result<(), String> {
    if label.is_empty()
        || matches!(label, "." | "..")
        || label.contains(['/', '\\'])
        || label.chars().any(char::is_control)
    {
        return Err(format!(
            "Invalid {kind} '{label}': use a plain name without path separators or dot segments."
        ));
    }
    Ok(())
}

fn color_output_enabled() -> bool {
    io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|term| term != "dumb")
            .unwrap_or(true)
}

fn colorize(text: &str, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SshCheck {
    /// `host` isn't a bare ssh-config alias (contains `@` or `:`).
    NotApplicable,
    /// No cache and the alias isn't in ssh config either — silent.
    NoCacheNoMatch,
    /// Alias is in ssh config but we never captured it.
    NoCacheMatch,
    /// Cached resolution matches current ssh config.
    Match,
    /// Cached resolution differs from current ssh config.
    CachedDrifted,
    /// We had a cache but the alias is no longer a Host entry.
    CachedMissing,
}

#[derive(Serialize)]
struct DoctorReport {
    alias: String,
    host: String,
    config: String,
    connected: Option<bool>,
    effective: BTreeMap<String, String>,
    message: Option<String>,
}

fn ssh_check_label(check: &SshCheck) -> &'static str {
    match check {
        SshCheck::NotApplicable => "not-applicable",
        SshCheck::NoCacheNoMatch => "ok",
        SshCheck::NoCacheMatch => "uncached",
        SshCheck::Match => "ok",
        SshCheck::CachedDrifted => "drifted",
        SshCheck::CachedMissing => "missing",
    }
}

fn doctor_report(
    alias: &str,
    server: &Server,
    connect: bool,
    options: &TransferOptions,
) -> DoctorReport {
    let check = check_ssh(server);
    let mut effective = BTreeMap::new();
    let mut config_cmd = Command::new("ssh");
    options.apply_ssh(&mut config_cmd);
    if let Ok(output) = config_cmd.arg("-G").arg("--").arg(&server.host).output()
        && output.status.success()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((key, value)) = line.split_once(' ')
                && matches!(
                    key,
                    "hostname" | "user" | "port" | "proxyjump" | "identityfile"
                )
            {
                effective.insert(key.to_string(), value.to_string());
            }
        }
    }

    if !connect {
        return DoctorReport {
            alias: alias.to_string(),
            host: server.host.clone(),
            config: ssh_check_label(&check).to_string(),
            connected: None,
            effective,
            message: None,
        };
    }

    let Some(path) = server.default_path() else {
        return DoctorReport {
            alias: alias.to_string(),
            host: server.host.clone(),
            config: ssh_check_label(&check).to_string(),
            connected: Some(false),
            effective,
            message: Some("default path is missing".to_string()),
        };
    };
    let quoted = remote_shell_path(path);
    let remote = format!(
        "test -d {quoted} && test -w {quoted} && \
         command -v stat >/dev/null && command -v find >/dev/null && \
         command -v grep >/dev/null && df -Pk {quoted} | tail -1"
    );
    let mut cmd = Command::new("ssh");
    options.apply_ssh(&mut cmd);
    let output = cmd
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "--"])
        .arg(&server.host)
        .arg(remote)
        .output();
    match output {
        Ok(output) if output.status.success() => DoctorReport {
            alias: alias.to_string(),
            host: server.host.clone(),
            config: ssh_check_label(&check).to_string(),
            connected: Some(true),
            effective,
            message: Some(String::from_utf8_lossy(&output.stdout).trim().to_string()),
        },
        Ok(output) => DoctorReport {
            alias: alias.to_string(),
            host: server.host.clone(),
            config: ssh_check_label(&check).to_string(),
            connected: Some(false),
            effective,
            message: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        },
        Err(e) => DoctorReport {
            alias: alias.to_string(),
            host: server.host.clone(),
            config: ssh_check_label(&check).to_string(),
            connected: Some(false),
            effective,
            message: Some(e.to_string()),
        },
    }
}

fn check_ssh(srv: &Server) -> SshCheck {
    if srv.host.contains('@') || srv.host.contains(':') {
        return SshCheck::NotApplicable;
    }
    let current = snd::ssh::lookup_alias(&srv.host).map(|h| SshResolved {
        hostname: h.hostname,
        user: h.user,
    });
    match (&srv.resolved, current) {
        (None, None) => SshCheck::NoCacheNoMatch,
        (None, Some(_)) => SshCheck::NoCacheMatch,
        (Some(_), None) => SshCheck::CachedMissing,
        (Some(cached), Some(current)) => {
            if cached == &current {
                SshCheck::Match
            } else {
                SshCheck::CachedDrifted
            }
        }
    }
}

struct ResolvedTarget {
    server_name: String,
    path_name: String,
    host: String,
    path: String,
}

impl ResolvedTarget {
    fn target(&self) -> String {
        format!("{}:{}", self.host, self.path)
    }
}

fn resolve_server_target(
    name: &str,
    servers: &BTreeMap<String, Server>,
    path_alias: Option<&str>,
) -> Result<ResolvedTarget, String> {
    let srv = servers
        .get(name)
        .ok_or_else(|| format!("Server '{name}' not found"))?;
    let (path_name, path) = match path_alias {
        Some(alias) => (
            alias.to_string(),
            srv.path_for(alias)
                .ok_or_else(|| format!("Path '{alias}' not found on '{name}'"))?
                .clone(),
        ),
        None => (
            srv.default.clone(),
            srv.default_path()
                .ok_or_else(|| format!("Server '{name}' has no paths configured"))?
                .clone(),
        ),
    };
    Ok(ResolvedTarget {
        server_name: name.to_string(),
        path_name,
        host: srv.host.clone(),
        path,
    })
}

fn resolve_group(
    group: &Group,
    servers: &BTreeMap<String, Server>,
) -> Result<Vec<ResolvedTarget>, String> {
    let mut out = Vec::with_capacity(group.targets.len());
    for raw in &group.targets {
        let gt = parse_group_target(raw);
        let resolved = resolve_server_target(gt.server, servers, gt.path_alias)?;
        out.push(resolved);
    }
    Ok(out)
}

fn expand_tildes(args: &mut [String]) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    for arg in args.iter_mut() {
        if let Some(rest) = arg.strip_prefix("~/") {
            *arg = home.join(rest).to_string_lossy().into_owned();
        } else if arg == "~" {
            *arg = home.to_string_lossy().into_owned();
        }
    }
}

fn uploadable_paths(args: &[String]) -> Vec<&String> {
    args.iter().collect()
}

fn print_target_block(target: &ResolvedTarget, label: &str, infos: &[RemoteFileInfo]) {
    if infos.is_empty() {
        return;
    }
    println!(
        "[{}] {}:{} — {label} ({}):",
        target.server_name,
        target.host,
        target.path,
        infos.len()
    );
    for info in infos {
        let kind = if info.is_dir { " (dir)" } else { "" };
        println!(
            "  {:<40}  {:>10}  {}{}",
            info.path,
            format_size(info.size),
            info.mtime,
            kind
        );
    }
}

fn resolve_remote_arg(base: &str, arg: &str) -> String {
    if arg.starts_with('/') || arg.starts_with('~') || arg.contains('/') {
        arg.to_string()
    } else {
        join_remote(base, arg)
    }
}

fn check_existing(
    target: &ResolvedTarget,
    local_args: &[String],
) -> Result<Vec<RemoteFileInfo>, String> {
    let mut remote_paths = Vec::new();
    for arg in uploadable_paths(local_args) {
        let Some(name) = destination_basename(arg) else {
            continue;
        };
        if !std::path::Path::new(arg).exists() {
            continue;
        }
        remote_paths.push(join_remote(&target.path, &name));
    }
    if remote_paths.is_empty() {
        return Ok(Vec::new());
    }
    stat_remote(&target.host, &remote_paths)
}

fn dispatch_send(
    cli_force: bool,
    no_check: bool,
    targets: Vec<ResolvedTarget>,
    args: Vec<String>,
    options: &TransferOptions,
) -> i32 {
    if !no_check && !cli_force {
        let mut any_existing = false;
        for target in &targets {
            match check_existing(target, &args) {
                Ok(existing) => {
                    if !existing.is_empty() {
                        any_existing = true;
                        if !options.json {
                            print_target_block(target, "file(s) already exist", &existing);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[{}] overwrite check failed: {e}", target.server_name);
                    eprintln!("Pass --no-check to skip the remote stat, or -f to force.");
                    return 1;
                }
            }
        }
        if any_existing && !options.dry_run && !confirm_for_output("Overwrite?", options.json) {
            eprintln!("Aborted.");
            return 1;
        }
    }

    if options.dry_run {
        if (options.atomic || options.resume || options.verify)
            && uploadable_paths(&args)
                .iter()
                .any(|path| !Path::new(path.as_str()).is_file())
        {
            eprintln!("--atomic, --resume, and --verify require regular files");
            return 1;
        }
        let plan: Vec<_> = targets
            .iter()
            .map(|target| {
                serde_json::json!({
                    "action": "upload",
                    "target": target.server_name,
                    "host": target.host,
                    "path": target.path,
                    "files": &args,
                    "atomic": options.atomic,
                    "verify": options.verify,
                    "resume": options.resume,
                    "backup": options.backup,
                    "backup_keep": options.backup_keep,
                })
            })
            .collect();
        if options.json {
            print_json("plan", true, &plan);
        } else {
            for item in &plan {
                println!(
                    "PLAN upload {} -> {}:{}",
                    args.join(" "),
                    item["host"].as_str().unwrap_or("?"),
                    item["path"].as_str().unwrap_or("?")
                );
            }
        }
        return 0;
    }

    let args = std::sync::Arc::new(args);
    let worker_options = options.clone();
    let results = run_parallel(targets, options.jobs, options.fail_fast, move |target| {
        upload_target(target, &args, &worker_options)
    });
    print_results("send", &results, options)
}

fn upload_target(
    target: ResolvedTarget,
    args: &[String],
    options: &TransferOptions,
) -> OperationResult {
    if !options.backup {
        return upload_target_without_backup(target, args, options);
    }

    let mut names = Vec::new();
    let mut collision_names = BTreeSet::new();
    for local in uploadable_paths(args) {
        if std::fs::symlink_metadata(local).is_err() {
            continue;
        }
        let Some(name) = destination_basename(local) else {
            return OperationResult::failure(
                target.server_name,
                "backup",
                0,
                format!("invalid local filename: {local}"),
            );
        };
        if name == ".snd" {
            return OperationResult::failure(
                target.server_name,
                "backup",
                0,
                "'.snd' is reserved for snd metadata; use --no-backup to send it directly",
            );
        }
        if !collision_names.insert(collision_key(Path::new(&name))) {
            return OperationResult::failure(
                target.server_name,
                "backup",
                0,
                format!("multiple inputs have the destination name '{name}'"),
            );
        }
        names.push(name);
    }
    // Preserve the historical behavior for nonexistent arguments: scp reports
    // the transfer error, while no empty rollback transaction is created.
    if names.is_empty() {
        return upload_target_without_backup(target, args, options);
    }

    let host = target.host.clone();
    let base = target.path.clone();
    let label = target.server_name.clone();
    let lock = match acquire_send_lock(&host, &base, options) {
        Ok(lock) => lock,
        Err(error) => return OperationResult::failure(label, "backup-lock", 1, error),
    };
    let backup = match prepare_send_backup(&host, &base, &names, options) {
        Ok(backup) => backup,
        Err(error) => {
            release_send_lock(&host, &lock, options);
            return OperationResult::failure(label, "backup", 1, error);
        }
    };

    let mut result = upload_target_without_backup(target, args, options);
    if result.success {
        match complete_send_backup(&host, &base, &backup, options.backup_keep, options) {
            Ok(()) => {
                result.message = Some(format!("rollback snapshot {}", backup.id));
                if !options.json && !options.progress {
                    eprintln!(
                        "[{}] rollback saved; undo with: snd rollback {}",
                        label, label
                    );
                }
            }
            Err(error) => {
                let restore = restore_send_backup(&host, &base, &backup, options);
                let message = match restore {
                    Ok(()) => {
                        format!("could not record rollback snapshot; upload was restored: {error}")
                    }
                    Err(restore_error) => format!(
                        "could not record rollback snapshot ({error}); automatic restore also failed: {restore_error}"
                    ),
                };
                result = OperationResult::failure(label, "backup", result.attempts, message);
            }
        }
    } else if let Err(restore_error) = restore_send_backup(&host, &base, &backup, options) {
        let transfer_error = result
            .message
            .take()
            .unwrap_or_else(|| "transfer failed".to_string());
        result.message = Some(format!(
            "{transfer_error}; automatic restore also failed: {restore_error}"
        ));
    }
    release_send_lock(&host, &lock, options);
    result
}

fn upload_target_without_backup(
    target: ResolvedTarget,
    args: &[String],
    options: &TransferOptions,
) -> OperationResult {
    if !(options.atomic || options.resume) {
        return upload_target_unlocked(target, args, options);
    }

    let mut locks = Vec::new();
    for local in uploadable_paths(args) {
        let Some(name) = destination_basename(local) else {
            continue;
        };
        let destination = join_remote(&target.path, &name);
        match acquire_remote_lock(&target.host, &destination, options) {
            Ok(lock) => locks.push(lock),
            Err(e) => {
                for lock in &locks {
                    release_remote_lock(&target.host, lock, options);
                }
                return OperationResult::failure(&target.server_name, "lock", 1, e);
            }
        }
    }
    let host = target.host.clone();
    let result = upload_target_unlocked(target, args, options);
    for lock in &locks {
        release_remote_lock(&host, lock, options);
    }
    result
}

fn upload_target_unlocked(
    target: ResolvedTarget,
    args: &[String],
    options: &TransferOptions,
) -> OperationResult {
    let label = target.server_name.clone();
    let files: Vec<&String> = uploadable_paths(args);
    let total_bytes: u64 = files
        .iter()
        .filter_map(|path| std::fs::metadata(path.as_str()).ok())
        .map(|metadata| metadata.len())
        .sum();
    if (options.atomic || options.resume || options.verify)
        && files.iter().any(|path| !Path::new(path.as_str()).is_file())
    {
        return OperationResult::failure(
            label,
            "upload",
            0,
            "--atomic, --resume, and --verify require regular files",
        );
    }

    if options.atomic || options.resume {
        let mut attempts = 0;
        for local in files {
            let Some(name) = destination_basename(local) else {
                return OperationResult::failure(
                    label,
                    "upload",
                    attempts,
                    "invalid local filename",
                );
            };
            let final_path = join_remote(&target.path, &name);
            let transfer_path = if options.atomic {
                if options.resume {
                    format!("{final_path}.snd-part")
                } else {
                    format!(
                        "{final_path}.snd-tmp-{}-{}",
                        std::process::id(),
                        TEMP_UPLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                    )
                }
            } else {
                final_path.clone()
            };
            if options.resume
                && let Err(e) =
                    validate_upload_resume(&target.host, Path::new(local), &transfer_path, options)
            {
                return OperationResult::failure(label, "resume-check", attempts, e);
            }
            if options.progress && !options.json {
                eprintln!("[{}] uploading {local}", target.server_name);
            }
            let (used, status) = retry_status(options.retries, || {
                if options.resume {
                    sftp_reput(&target.host, Path::new(local), &transfer_path, options)
                } else {
                    let mut cmd = Command::new("scp");
                    options.apply_scp(&mut cmd);
                    if options.json {
                        cmd.stdout(Stdio::null());
                    }
                    cmd.arg("--").arg(local).arg(format!(
                        "{}:{}",
                        target.host,
                        scp_literal_remote_path(&transfer_path)
                    ));
                    cmd.status()
                }
            });
            attempts += used;
            match status {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    cleanup_atomic_temp(&target.host, &transfer_path, options);
                    return OperationResult::failure(
                        label,
                        "upload",
                        attempts,
                        format!("transfer exited with {}", status.code().unwrap_or(1)),
                    );
                }
                Err(e) => {
                    cleanup_atomic_temp(&target.host, &transfer_path, options);
                    return OperationResult::failure(label, "upload", attempts, e.to_string());
                }
            }
            if options.verify {
                let local_hash = match local_sha256(Path::new(local)) {
                    Ok(hash) => hash,
                    Err(e) => {
                        cleanup_atomic_temp(&target.host, &transfer_path, options);
                        return OperationResult::failure(label, "verify", attempts, e);
                    }
                };
                match remote_sha256(&target.host, &transfer_path, options) {
                    Ok(remote_hash) if remote_hash == local_hash => {}
                    Ok(_) => {
                        cleanup_atomic_temp(&target.host, &transfer_path, options);
                        return OperationResult::failure(
                            label,
                            "verify",
                            attempts,
                            format!("checksum mismatch for {local}"),
                        );
                    }
                    Err(e) => {
                        cleanup_atomic_temp(&target.host, &transfer_path, options);
                        return OperationResult::failure(label, "verify", attempts, e);
                    }
                }
            }
            if options.atomic {
                match remote_rename(&target.host, &transfer_path, &final_path, options) {
                    Ok(status) if status.success() => {}
                    Ok(status) => {
                        cleanup_atomic_temp(&target.host, &transfer_path, options);
                        return OperationResult::failure(
                            label,
                            "rename",
                            attempts,
                            format!("remote rename exited with {}", status.code().unwrap_or(1)),
                        );
                    }
                    Err(e) => {
                        cleanup_atomic_temp(&target.host, &transfer_path, options);
                        return OperationResult::failure(label, "rename", attempts, e.to_string());
                    }
                }
            }
        }
        return OperationResult::success(label, "upload", attempts.max(1)).with_bytes(total_bytes);
    }

    let dest = target.target();
    if !options.json {
        println!("scp {} -> {dest}", args.join(" "));
        if options.progress {
            eprintln!("[{}] transfer started", target.server_name);
        }
    }
    let (attempts, status) = retry_status(options.retries, || {
        let mut cmd = Command::new("scp");
        options.apply_scp(&mut cmd);
        if files.iter().any(|path| Path::new(path.as_str()).is_dir()) {
            cmd.arg("-r");
        }
        if options.json {
            cmd.stdout(Stdio::null());
        }
        cmd.arg("--").args(args).arg(&dest).status()
    });
    match status {
        Ok(status) if status.success() => {
            if options.verify {
                for local in files {
                    let Some(name) = destination_basename(local) else {
                        continue;
                    };
                    let remote = join_remote(&target.path, &name);
                    let local_hash = match local_sha256(Path::new(local)) {
                        Ok(hash) => hash,
                        Err(e) => return OperationResult::failure(label, "verify", attempts, e),
                    };
                    match remote_sha256(&target.host, &remote, options) {
                        Ok(hash) if hash == local_hash => {}
                        Ok(_) => {
                            return OperationResult::failure(
                                label,
                                "verify",
                                attempts,
                                format!("checksum mismatch for {local}"),
                            );
                        }
                        Err(e) => return OperationResult::failure(label, "verify", attempts, e),
                    }
                }
            }
            OperationResult::success(label, "upload", attempts).with_bytes(total_bytes)
        }
        Ok(status) => OperationResult::failure(
            label,
            "upload",
            attempts,
            format!("scp exited with {}", status.code().unwrap_or(1)),
        ),
        Err(e) => OperationResult::failure(label, "upload", attempts, e.to_string()),
    }
}

fn cleanup_atomic_temp(host: &str, path: &str, options: &TransferOptions) {
    if options.atomic && !options.resume {
        let _ = rm_remote(host, &[path.to_string()], false);
    }
}

fn dispatch_delete(
    recursive: bool,
    targets: Vec<ResolvedTarget>,
    files: Vec<String>,
    options: &TransferOptions,
) -> i32 {
    if options.atomic || options.resume || options.verify {
        eprintln!("--atomic, --resume, and --verify are transfer-only options.");
        return 1;
    }
    if files.is_empty() {
        eprintln!("No files specified for delete.");
        return 1;
    }

    let mut per_target: Vec<(ResolvedTarget, Vec<RemoteFileInfo>, Vec<RemoteFileInfo>)> =
        Vec::new();
    for target in targets {
        let mut remote_paths = Vec::new();
        for f in &files {
            remote_paths.push(resolve_remote_arg(&target.path, f));
        }
        let infos = match stat_remote(&target.host, &remote_paths) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("[{}] stat failed: {e}", target.server_name);
                eprintln!("Refusing to delete without a successful remote stat.");
                return 1;
            }
        };
        let found_paths: std::collections::HashSet<&str> =
            infos.iter().map(|i| i.path.as_str()).collect();
        for m in remote_paths
            .iter()
            .filter(|p| !found_paths.contains(p.as_str()))
        {
            if !options.json {
                println!(
                    "[{}] {}:{} — not found: {m}",
                    target.server_name, target.host, target.path
                );
            }
        }
        let (dirs, files_only): (Vec<_>, Vec<_>) = infos.into_iter().partition(|i| i.is_dir);
        per_target.push((target, files_only, dirs));
    }

    if !recursive {
        let mut had_dirs = false;
        for (target, _, dirs) in &per_target {
            for d in dirs {
                had_dirs = true;
                eprintln!(
                    "[{}] refusing to delete directory {} (pass -r/--recursive to allow)",
                    target.server_name, d.path
                );
            }
        }
        if had_dirs {
            for (_, _, dirs) in per_target.iter_mut() {
                dirs.clear();
            }
        }
    }

    let any_files = per_target.iter().any(|(_, f, _)| !f.is_empty());
    let any_dirs = per_target.iter().any(|(_, _, d)| !d.is_empty());
    if !any_files && !any_dirs {
        eprintln!("Nothing to delete.");
        return 1;
    }

    if !options.json {
        for (target, files_only, dirs) in &per_target {
            print_target_block(target, "files to delete", files_only);
            print_target_block(target, "DIRECTORIES to delete (recursive)", dirs);
        }
    }

    if options.dry_run {
        if options.json {
            let plan: Vec<_> = per_target
                .iter()
                .flat_map(|(target, files, dirs)| {
                    files.iter().chain(dirs).map(|info| {
                        serde_json::json!({
                            "action": "delete",
                            "target": target.server_name,
                            "host": target.host,
                            "path": info.path,
                            "recursive": info.is_dir,
                        })
                    })
                })
                .collect();
            print_json("delete-plan", true, &plan);
        }
        return 0;
    }

    let prompt = if any_dirs {
        "This will recursively delete directories. Proceed?"
    } else {
        "Delete these?"
    };
    if !confirm_for_output(prompt, options.json) {
        eprintln!("Aborted.");
        return 1;
    }

    let results = run_parallel(
        per_target,
        options.jobs,
        options.fail_fast,
        move |(target, files_only, dirs)| delete_target(target, files_only, dirs),
    );
    print_results("delete", &results, options)
}

fn delete_target(
    target: ResolvedTarget,
    files_only: Vec<RemoteFileInfo>,
    dirs: Vec<RemoteFileInfo>,
) -> OperationResult {
    let label = target.server_name.clone();
    let mut failed = Vec::new();
    let mut attempts = 0;
    {
        if !files_only.is_empty() {
            let paths: Vec<String> = files_only.into_iter().map(|i| i.path).collect();
            attempts += 1;
            match rm_remote(&target.host, &paths, false) {
                Ok(status) => {
                    if !status.success() {
                        failed.push(format!("rm exited with {}", status.code().unwrap_or(1)));
                    }
                }
                Err(e) => {
                    failed.push(format!("ssh rm failed: {e}"));
                }
            }
        }
        if !dirs.is_empty() {
            let paths: Vec<String> = dirs.into_iter().map(|i| i.path).collect();
            attempts += 1;
            match rm_remote(&target.host, &paths, true) {
                Ok(status) => {
                    if !status.success() {
                        failed.push(format!("rm -r exited with {}", status.code().unwrap_or(1)));
                    }
                }
                Err(e) => {
                    failed.push(format!("ssh rm -r failed: {e}"));
                }
            }
        }
    }
    if failed.is_empty() {
        OperationResult::success(label, "delete", attempts.max(1))
    } else {
        OperationResult::failure(label, "delete", attempts.max(1), failed.join("; "))
    }
}

const MAX_FIND_LISTED: usize = 200;

fn dispatch_find(
    grep: bool,
    regex: bool,
    case_sensitive: bool,
    depth: Option<u32>,
    targets: Vec<ResolvedTarget>,
    pattern: &str,
    json: bool,
) -> i32 {
    let color = color_output_enabled();
    let mut worst = 0;
    let mut reports = Vec::new();
    for target in &targets {
        if grep {
            match grep_remote(
                &target.host,
                &target.path,
                pattern,
                regex,
                case_sensitive,
                color,
            ) {
                Ok(lines) => {
                    if json {
                        reports.push(serde_json::json!({
                            "target": target.server_name,
                            "host": target.host,
                            "path": target.path,
                            "mode": "grep",
                            "matches": lines,
                        }));
                        continue;
                    }
                    println!("[{}] {}:{}", target.server_name, target.host, target.path);
                    if lines.is_empty() {
                        println!("  no matches");
                    } else {
                        for line in &lines {
                            println!("  {line}");
                        }
                    }
                }
                Err(e) => {
                    if json {
                        reports.push(serde_json::json!({
                            "target": target.server_name,
                            "host": target.host,
                            "path": target.path,
                            "mode": "grep",
                            "error": e,
                        }));
                    } else {
                        eprintln!("[{}] search failed: {e}", target.server_name);
                    }
                    worst = 1;
                }
            }
            continue;
        }

        match find_remote(
            &target.host,
            &target.path,
            pattern,
            regex,
            case_sensitive,
            depth,
        ) {
            Ok(paths) => {
                if json {
                    reports.push(serde_json::json!({
                        "target": target.server_name,
                        "host": target.host,
                        "path": target.path,
                        "mode": "find",
                        "matches": paths,
                    }));
                    continue;
                }
                if paths.is_empty() {
                    println!(
                        "[{}] {}:{} — no matches",
                        target.server_name, target.host, target.path
                    );
                    continue;
                }
                let total = paths.len();
                let shown = &paths[..total.min(MAX_FIND_LISTED)];
                let truncated = if total > shown.len() {
                    format!(" (showing {})", shown.len())
                } else {
                    String::new()
                };
                println!(
                    "[{}] {}:{} — {total} match(es){truncated}:",
                    target.server_name, target.host, target.path
                );
                match stat_remote(&target.host, shown) {
                    Ok(infos) if !infos.is_empty() => {
                        for info in &infos {
                            let kind = if info.is_dir { " (dir)" } else { "" };
                            let path = format!("{:<40}", info.path);
                            println!(
                                "  {}  {:>10}  {}{}",
                                colorize(&path, "36", color),
                                format_size(info.size),
                                info.mtime,
                                kind
                            );
                        }
                    }
                    _ => {
                        for p in shown {
                            println!("  {}", colorize(p, "36", color));
                        }
                    }
                }
                if total > shown.len() {
                    println!(
                        "  … {} more (narrow with a path-alias, -p, or --depth)",
                        total - shown.len()
                    );
                }
            }
            Err(e) => {
                if json {
                    reports.push(serde_json::json!({
                        "target": target.server_name,
                        "host": target.host,
                        "path": target.path,
                        "mode": "find",
                        "error": e,
                    }));
                } else {
                    eprintln!("[{}] search failed: {e}", target.server_name);
                }
                worst = 1;
            }
        }
    }
    if json {
        print_json("find", worst == 0, &reports);
    }
    worst
}

fn dispatch_ls(targets: Vec<ResolvedTarget>) -> i32 {
    let color = color_output_enabled();
    let multi = targets.len() > 1;
    let mut worst = 0;
    for target in &targets {
        if multi {
            println!("[{}] {}:{}", target.server_name, target.host, target.path);
        }
        match ls_remote(&target.host, &target.path, color) {
            Ok(status) => {
                let code = status.code().unwrap_or(1);
                if code != 0 && code > worst {
                    worst = code;
                }
            }
            Err(e) => {
                eprintln!("[{}] ls failed: {e}", target.server_name);
                worst = 1;
            }
        }
        if multi {
            println!();
        }
    }
    worst
}

fn dispatch_cat(targets: Vec<ResolvedTarget>, files: Vec<String>) -> i32 {
    let color = color_output_enabled();
    if files.is_empty() {
        eprintln!("No files specified.");
        return 1;
    }
    let multi = targets.len() > 1;
    let mut worst = 0;
    for target in &targets {
        let paths: Vec<String> = files
            .iter()
            .map(|f| resolve_remote_arg(&target.path, f))
            .collect();
        if multi {
            println!("[{}] {}:{}", target.server_name, target.host, target.path);
        }
        match cat_remote(&target.host, &paths, color) {
            Ok(status) => {
                let code = status.code().unwrap_or(1);
                if code != 0 && code > worst {
                    worst = code;
                }
            }
            Err(e) => {
                eprintln!("[{}] cat failed: {e}", target.server_name);
                worst = 1;
            }
        }
    }
    worst
}

#[derive(Serialize)]
struct DiffEntry {
    target: String,
    local: String,
    remote: String,
    status: String,
    local_size: Option<u64>,
    remote_size: Option<u64>,
}

fn dispatch_diff(
    targets: Vec<ResolvedTarget>,
    files: Vec<String>,
    hash: bool,
    options: &TransferOptions,
) -> i32 {
    let mut entries = Vec::new();
    for target in &targets {
        let mut pairs = Vec::new();
        for local in &files {
            let Some(name) = destination_basename(local) else {
                continue;
            };
            pairs.push((local.clone(), join_remote(&target.path, &name)));
        }
        let remote_paths: Vec<String> = pairs.iter().map(|(_, remote)| remote.clone()).collect();
        let remote_infos = match stat_remote(&target.host, &remote_paths) {
            Ok(infos) => infos,
            Err(e) => {
                eprintln!("[{}] diff stat failed: {e}", target.server_name);
                return 1;
            }
        };
        let remote_by_path: BTreeMap<&str, &RemoteFileInfo> = remote_infos
            .iter()
            .map(|info| (info.path.as_str(), info))
            .collect();
        for (local, remote) in pairs {
            let local_meta = std::fs::metadata(&local).ok();
            let remote_info = remote_by_path.get(remote.as_str()).copied();
            let mut status = match (&local_meta, remote_info) {
                (None, _) => "missing-local",
                (Some(_), None) => "missing-remote",
                (Some(local), Some(remote)) if local.len() == remote.size => "same",
                _ => "different",
            }
            .to_string();
            if hash && status == "same" {
                match (
                    local_sha256(Path::new(&local)),
                    remote_sha256(&target.host, &remote, options),
                ) {
                    (Ok(local_hash), Ok(remote_hash)) if local_hash == remote_hash => {}
                    (Ok(_), Ok(_)) => status = "different".to_string(),
                    (Err(e), _) | (_, Err(e)) => status = format!("error: {e}"),
                }
            }
            entries.push(DiffEntry {
                target: target.server_name.clone(),
                local: local.clone(),
                remote,
                status,
                local_size: local_meta.map(|meta| meta.len()),
                remote_size: remote_info.map(|info| info.size),
            });
        }
    }
    if options.json {
        print_json(
            "diff",
            entries.iter().all(|entry| entry.status == "same"),
            &entries,
        );
    } else {
        for entry in &entries {
            println!(
                "{:<15} {:<15} {} -> {}",
                entry.status, entry.target, entry.local, entry.remote
            );
        }
    }
    if entries.iter().all(|entry| entry.status == "same") {
        0
    } else {
        1
    }
}

fn rsync_target(
    target: ResolvedTarget,
    source: &str,
    delete: bool,
    filters: &SyncFilters,
    options: &TransferOptions,
) -> OperationResult {
    let label = target.server_name.clone();
    let destination = format!("{}:{}/", target.host, target.path.trim_end_matches('/'));
    let (attempts, status) = retry_status(options.retries, || {
        let mut cmd = Command::new("rsync");
        cmd.args(["-rlt", "-s", "--partial-dir=.snd-partial"]);
        if options.preserve {
            cmd.arg("-p");
        }
        if options.compress {
            cmd.arg("-z");
        }
        if let Some(limit) = options.limit {
            cmd.arg(format!("--bwlimit={}", (limit / 8).max(1)));
        }
        if delete {
            cmd.arg("--delete");
        }
        apply_sync_filters(&mut cmd, filters);
        if options.progress && !options.json {
            cmd.arg("--info=progress2");
        }
        if options.json {
            cmd.stdout(Stdio::null());
        }
        cmd.arg("-e").arg(options.rsync_shell());
        cmd.arg("--")
            .arg(format!("{}/", source.trim_end_matches('/')))
            .arg(&destination)
            .status()
    });
    match status {
        Ok(status) if status.success() => OperationResult::success(label, "sync", attempts),
        Ok(status) => OperationResult::failure(
            label,
            "sync",
            attempts,
            format!("rsync exited with {}", status.code().unwrap_or(1)),
        ),
        Err(e) => OperationResult::failure(label, "sync", attempts, e.to_string()),
    }
}

#[derive(Clone)]
struct SyncFilters {
    include: Vec<String>,
    exclude: Vec<String>,
    ignore_file: Option<String>,
}

fn apply_sync_filters(command: &mut Command, filters: &SyncFilters) {
    // Rollback metadata belongs to snd, not to the synchronized source tree.
    command.arg("--exclude=/.snd/");
    for pattern in &filters.include {
        command.arg("--include").arg(pattern);
    }
    for pattern in &filters.exclude {
        command.arg("--exclude").arg(pattern);
    }
    if let Some(path) = &filters.ignore_file {
        command.arg("--exclude-from").arg(path);
        command.arg("--exclude=.sndignore");
    }
}

struct SyncRequest {
    source: String,
    delete: bool,
    force: bool,
    include: Vec<String>,
    exclude: Vec<String>,
    ignore_file: Option<String>,
    no_ignore: bool,
}

#[derive(Serialize)]
struct SyncPlan {
    target: String,
    host: String,
    destination: String,
    changes: Vec<String>,
}

fn dispatch_sync(
    targets: Vec<ResolvedTarget>,
    request: SyncRequest,
    options: &TransferOptions,
) -> i32 {
    let SyncRequest {
        source,
        delete,
        force,
        include,
        exclude,
        ignore_file,
        no_ignore,
    } = request;
    if options.atomic || options.resume || options.verify {
        eprintln!("--atomic, --resume, and --verify are file-transfer options, not sync options.");
        return 1;
    }
    if !Path::new(&source).is_dir() {
        eprintln!("Sync source must be a directory: {source}");
        return 1;
    }
    let ignore_file = if no_ignore {
        ignore_file
    } else {
        ignore_file.or_else(|| {
            let candidate = Path::new(&source).join(".sndignore");
            candidate
                .is_file()
                .then(|| candidate.to_string_lossy().into_owned())
        })
    };
    if let Some(path) = ignore_file.as_deref()
        && !Path::new(path).is_file()
    {
        eprintln!("Sync ignore file does not exist: {path}");
        return 1;
    }
    let filters = SyncFilters {
        include,
        exclude,
        ignore_file,
    };
    let mut plans = Vec::new();
    for target in &targets {
        let destination = format!("{}:{}/", target.host, target.path.trim_end_matches('/'));
        let mut cmd = Command::new("rsync");
        cmd.args(["-rlt", "-s", "--dry-run", "--itemize-changes"]);
        if delete {
            cmd.arg("--delete");
        }
        apply_sync_filters(&mut cmd, &filters);
        cmd.arg("-e")
            .arg(options.rsync_shell())
            .arg("--")
            .arg(format!("{}/", source.trim_end_matches('/')))
            .arg(&destination);
        match cmd.output() {
            Ok(output) if output.status.success() => {
                let plan = String::from_utf8_lossy(&output.stdout);
                let changes: Vec<String> = plan
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect();
                if !changes.is_empty() && !options.json {
                    println!("[{}] sync plan:\n{}", target.server_name, plan.trim());
                }
                plans.push(SyncPlan {
                    target: target.server_name.clone(),
                    host: target.host.clone(),
                    destination,
                    changes,
                });
            }
            Ok(output) => {
                eprintln!(
                    "[{}] rsync planning failed: {}",
                    target.server_name,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                return 1;
            }
            Err(e) => {
                eprintln!("Failed to run rsync: {e}");
                return 1;
            }
        }
    }
    let changed = plans.iter().any(|plan| !plan.changes.is_empty());
    if options.dry_run && options.json {
        print_json("sync-plan", true, &plans);
        return 0;
    }
    if !changed {
        if options.json {
            print_json("sync", true, &Vec::<OperationResult>::new());
        } else {
            println!("Already synchronized.");
        }
        return 0;
    }
    if options.dry_run {
        return 0;
    }
    if !force
        && !confirm_for_output(
            if delete {
                "Apply sync plan, including remote deletions?"
            } else {
                "Apply sync plan?"
            },
            options.json,
        )
    {
        eprintln!("Aborted.");
        return 1;
    }
    let worker_options = options.clone();
    let worker_source = source.clone();
    let worker_filters = filters.clone();
    let results = run_parallel(targets, options.jobs, options.fail_fast, move |target| {
        rsync_target(
            target,
            &worker_source,
            delete,
            &worker_filters,
            &worker_options,
        )
    });
    print_results("sync", &results, options)
}

fn deploy_target(
    mut target: ResolvedTarget,
    files: &[String],
    release: &str,
    keep: usize,
    options: &TransferOptions,
) -> OperationResult {
    let label = target.server_name.clone();
    let base = target.path.clone();
    let host = target.host.clone();
    let total_bytes = files
        .iter()
        .filter_map(|file| std::fs::metadata(file).ok())
        .map(|metadata| metadata.len())
        .sum();
    let lock = match acquire_deploy_lock(&host, &base, options) {
        Ok(lock) => lock,
        Err(error) => return OperationResult::failure(label, "deploy-lock", 1, error),
    };
    let result = (|| {
        let directory = prepare_release(&host, &base, release, options)?;
        target.path = directory;
        let mut transfer_options = options.clone();
        transfer_options.atomic = true;
        transfer_options.verify = true;
        transfer_options.backup = false;
        let upload = upload_target(target, files, &transfer_options);
        if !upload.success {
            if !options.resume {
                remove_release(&host, &base, release, options);
            }
            return Err(upload
                .message
                .unwrap_or_else(|| "release upload failed".to_string()));
        }
        complete_release(&host, &base, release, options)?;
        activate_release(&host, &base, release, options)?;
        Ok(upload.attempts)
    })();
    release_deploy_lock(&host, &lock, options);
    match result {
        Ok(attempts) => {
            // Pruning is best-effort after activation; it never invalidates the new release.
            if let Err(error) = prune_releases(&host, &base, keep, options) {
                eprintln!("[{label}] release pruning warning: {error}");
            }
            OperationResult::success(label, "deploy", attempts).with_bytes(total_bytes)
        }
        Err(error) => OperationResult::failure(label, "deploy", 1, error),
    }
}

#[derive(Serialize)]
struct ReleasePlan {
    target: String,
    host: String,
    base: String,
    release: String,
    directory: String,
    files: Vec<String>,
}

fn dispatch_deploy(
    targets: Vec<ResolvedTarget>,
    files: Vec<String>,
    release: String,
    keep: usize,
    options: &TransferOptions,
) -> i32 {
    if files.is_empty() {
        eprintln!("No release files specified.");
        return 1;
    }
    if let Err(error) = validate_release_name(&release) {
        eprintln!("{error}");
        return 1;
    }
    if let Some(file) = files.iter().find(|file| !Path::new(file).is_file()) {
        eprintln!("Release inputs must be regular files: {file}");
        return 1;
    }
    let mut release_names = BTreeSet::new();
    for file in &files {
        let Some(name) = destination_basename(file) else {
            eprintln!("Invalid release filename: {file}");
            return 1;
        };
        if name == ".snd-complete" {
            eprintln!("'.snd-complete' is reserved for release metadata.");
            return 1;
        }
        if !release_names.insert(collision_key(Path::new(&name))) {
            eprintln!("Release filename collision: multiple inputs are named '{name}'.");
            return 1;
        }
    }
    let plans: Vec<_> = targets
        .iter()
        .map(|target| ReleasePlan {
            target: target.server_name.clone(),
            host: target.host.clone(),
            base: target.path.clone(),
            release: release.clone(),
            directory: release_directory(&target.path, &release),
            files: files.clone(),
        })
        .collect();
    if options.dry_run {
        if options.json {
            print_json("deploy-plan", true, &plans);
        } else {
            for plan in &plans {
                println!(
                    "PLAN deploy release {}: {} -> {}:{} (activate .snd/current)",
                    plan.release,
                    plan.files.join(" "),
                    plan.host,
                    plan.directory
                );
            }
        }
        return 0;
    }
    let worker_files = std::sync::Arc::new(files);
    let worker_release = release.clone();
    let worker_options = options.clone();
    let results = run_parallel(targets, options.jobs, options.fail_fast, move |target| {
        deploy_target(
            target,
            &worker_files,
            &worker_release,
            keep,
            &worker_options,
        )
    });
    print_results("deploy", &results, options)
}

fn rollback_target(
    target: ResolvedTarget,
    requested: Option<&str>,
    release_only: bool,
    names: &[String],
    options: &TransferOptions,
) -> OperationResult {
    let label = target.server_name.clone();
    if !names.is_empty() {
        let send_lock = match acquire_send_lock(&target.host, &target.path, options) {
            Ok(lock) => lock,
            Err(error) => return OperationResult::failure(label, "rollback-lock", 1, error),
        };
        let restored = rollback_named_sends(&target.host, &target.path, names, options);
        release_send_lock(&target.host, &send_lock, options);
        return match restored {
            Ok(restored) => OperationResult {
                message: Some(format!(
                    "restored {}",
                    restored
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                ..OperationResult::success(label, "rollback-files", 1)
            },
            Err(error) => OperationResult::failure(label, "rollback-files", 1, error),
        };
    }
    if requested.is_none() && !release_only {
        let send_lock = match acquire_send_lock(&target.host, &target.path, options) {
            Ok(lock) => lock,
            Err(error) => return OperationResult::failure(label, "rollback-lock", 1, error),
        };
        let direct = rollback_latest_send(&target.host, &target.path, options);
        release_send_lock(&target.host, &send_lock, options);
        match direct {
            Ok(SendRollback::Restored(id)) => {
                return OperationResult {
                    message: Some(format!("restored direct send {id}")),
                    ..OperationResult::success(label, "rollback-send", 1)
                };
            }
            Ok(SendRollback::None) => {}
            Err(error) => return OperationResult::failure(label, "rollback-send", 1, error),
        }
    }

    let lock = match acquire_deploy_lock(&target.host, &target.path, options) {
        Ok(lock) => lock,
        Err(error) => return OperationResult::failure(label, "rollback-lock", 1, error),
    };
    let result = rollback_release(&target.host, &target.path, requested, options);
    release_deploy_lock(&target.host, &lock, options);
    match result {
        Ok(release) => OperationResult {
            message: Some(format!("activated release {release}")),
            ..OperationResult::success(label, "rollback", 1)
        },
        Err(error) => {
            let error = if requested.is_none() && !release_only {
                format!("no direct-send backup or usable previous release: {error}")
            } else {
                error
            };
            OperationResult::failure(label, "rollback", 1, error)
        }
    }
}

fn dispatch_rollback(
    targets: Vec<ResolvedTarget>,
    requested: Option<String>,
    release_only: bool,
    names: Vec<String>,
    options: &TransferOptions,
) -> i32 {
    if let Some(release) = requested.as_deref()
        && let Err(error) = validate_release_name(release)
    {
        eprintln!("{error}");
        return 1;
    }
    if !names.is_empty() && (requested.is_some() || release_only) {
        eprintln!("FILE arguments cannot be combined with --release or --to.");
        return 1;
    }
    let mut unique_names = BTreeSet::new();
    let mut normalized_names = Vec::with_capacity(names.len());
    for value in names {
        let Some(name) = destination_basename(&value) else {
            eprintln!("Invalid rollback filename: {value}");
            return 1;
        };
        if name.is_empty()
            || matches!(name.as_str(), "." | ".." | ".snd")
            || name.contains(['/', '\n', '\r'])
        {
            eprintln!("Invalid rollback filename: {value}");
            return 1;
        }
        if !unique_names.insert(collision_key(Path::new(&name))) {
            eprintln!("Duplicate rollback filename: {name}");
            return 1;
        }
        normalized_names.push(name);
    }
    if options.dry_run {
        let plans: Vec<_> = targets
            .iter()
            .map(|target| {
                serde_json::json!({
                    "target": target.server_name,
                    "host": target.host,
                    "base": target.path,
                    "mode": if !normalized_names.is_empty() { "selected-direct-send-files" } else if requested.is_some() || release_only { "release" } else { "latest-direct-send-or-release" },
                    "release": requested.as_deref().unwrap_or("previous"),
                    "files": &normalized_names,
                })
            })
            .collect();
        if options.json {
            print_json("rollback-plan", true, &plans);
        } else {
            for plan in &plans {
                println!(
                    "PLAN rollback {} ({})",
                    plan["target"].as_str().unwrap_or("?"),
                    if !normalized_names.is_empty() {
                        format!("restore {}", normalized_names.join(", "))
                    } else if requested.is_some() || release_only {
                        format!("release {}", plan["release"].as_str().unwrap_or("previous"))
                    } else {
                        "latest direct send; fall back to previous release".to_string()
                    }
                );
            }
        }
        return 0;
    }
    let worker_options = options.clone();
    let worker_names = std::sync::Arc::new(normalized_names);
    let results = run_parallel(targets, options.jobs, options.fail_fast, move |target| {
        rollback_target(
            target,
            requested.as_deref(),
            release_only,
            &worker_names,
            &worker_options,
        )
    });
    print_results("rollback", &results, options)
}

#[derive(Serialize)]
struct TargetReleaseState {
    target: String,
    host: String,
    base: String,
    state: snd::deploy::ReleaseState,
}

fn dispatch_releases(targets: Vec<ResolvedTarget>, options: &TransferOptions) -> i32 {
    let mut reports = Vec::new();
    for target in targets {
        match release_state(&target.host, &target.path, options) {
            Ok(state) => reports.push(TargetReleaseState {
                target: target.server_name,
                host: target.host,
                base: target.path,
                state,
            }),
            Err(error) => {
                eprintln!("[{}] failed to list releases: {error}", target.server_name);
                return 1;
            }
        }
    }
    if options.json {
        print_json("releases", true, &reports);
    } else {
        for report in &reports {
            println!("[{}] {}:{}", report.target, report.host, report.base);
            println!(
                "  active:   {}",
                report.state.active.as_deref().unwrap_or("(none)")
            );
            println!(
                "  previous: {}",
                report.state.previous.as_deref().unwrap_or("(none)")
            );
            for release in &report.state.releases {
                println!("  - {release}");
            }
        }
    }
    0
}

#[derive(Serialize)]
struct TargetSendHistory {
    target: String,
    host: String,
    base: String,
    transactions: Vec<SendHistoryEntry>,
}

fn dispatch_history(
    targets: Vec<ResolvedTarget>,
    filter: Option<String>,
    options: &TransferOptions,
) -> i32 {
    let filter = match filter {
        Some(value) => match destination_basename(&value) {
            Some(name) => Some(name),
            None => {
                eprintln!("Invalid history filename: {value}");
                return 1;
            }
        },
        None => None,
    };
    let mut reports = Vec::new();
    for target in targets {
        match send_history(&target.host, &target.path, filter.as_deref(), options) {
            Ok(transactions) => reports.push(TargetSendHistory {
                target: target.server_name,
                host: target.host,
                base: target.path,
                transactions,
            }),
            Err(error) => {
                eprintln!("[{}] failed to read history: {error}", target.server_name);
                return 1;
            }
        }
    }
    if options.json {
        print_json("history", true, &reports);
        return 0;
    }
    for report in reports {
        println!(
            "[{}] {}:{} — rollback history",
            report.target, report.host, report.base
        );
        if report.transactions.is_empty() {
            println!("  No matching transactions.");
            continue;
        }
        for transaction in report.transactions {
            let legacy = if transaction.storage == "legacy-target" {
                " (legacy destination storage)"
            } else {
                ""
            };
            println!("  {}{legacy}", transaction.id);
            for file in transaction.files {
                let effect = if file.previous_state == "present" {
                    "restore saved version"
                } else {
                    "remove newly created destination"
                };
                println!("    {:<32} {effect}", file.name);
            }
        }
    }
    0
}

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();
    let options = transfer_options(&cli);
    snd::remote::set_transfer_options(options.clone());

    match cli.command {
        Some(Cmd::Plan { target, mut files }) => {
            let cfg = load_or_exit();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, cli.path.is_none())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if cli.path.is_none() {
                apply_positional_path_override(&mut resolved, &mut files, true);
            }
            if let Some(path) = cli.path.as_deref() {
                apply_path_override(&mut resolved, path);
            }
            if files.is_empty() {
                eprintln!("No files specified for plan.");
                std::process::exit(1);
            }
            let (resolved, _) = expand_or_exit(resolved);
            expand_tildes(&mut files);
            let mut plan_options = options.clone();
            plan_options.dry_run = true;
            std::process::exit(dispatch_send(
                cli.force,
                cli.no_check,
                resolved,
                files,
                &plan_options,
            ));
        }
        Some(Cmd::Add { alias, host, path }) => {
            validate_alias(&alias, "server alias").unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let mut cfg = load_mutable_or_exit(cli.local);
            if cfg.servers.contains_key(&alias) {
                eprintln!(
                    "Server '{alias}' already exists. Use 'snd edit {alias} <host>' to change host, or 'snd add-path' to add a path."
                );
                std::process::exit(1);
            }
            if cfg.groups.contains_key(&alias) {
                eprintln!("'{alias}' is already used as a group name.");
                std::process::exit(1);
            }
            let path = path.unwrap_or_else(|| "~".to_string()).replace("\\~", "~");
            let mut paths = BTreeMap::new();
            paths.insert("default".to_string(), path.clone());
            let resolved = snd::ssh::lookup_alias(&host).map(|h| SshResolved {
                hostname: h.hostname,
                user: h.user,
            });
            cfg.servers.insert(
                alias.clone(),
                Server {
                    host: host.clone(),
                    default: "default".to_string(),
                    paths,
                    resolved,
                },
            );
            save_mutable_or_exit(&cfg, cli.local);
            println!("Added: {alias} -> {host}:{path}");
        }
        Some(Cmd::Remove { alias }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            if cfg.servers.remove(&alias).is_none() {
                eprintln!("Server '{alias}' not found.");
                std::process::exit(1);
            }
            for g in cfg.groups.values_mut() {
                g.targets.retain(|t| {
                    let gt = parse_group_target(t);
                    gt.server != alias
                });
            }
            cfg.groups.retain(|_, g| !g.targets.is_empty());
            save_mutable_or_exit(&cfg, cli.local);
            println!("Removed: {alias}");
        }
        Some(Cmd::Edit { alias, host }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            let resolved = snd::ssh::lookup_alias(&host).map(|h| SshResolved {
                hostname: h.hostname,
                user: h.user,
            });
            let Some(srv) = cfg.servers.get_mut(&alias) else {
                eprintln!("Server '{alias}' not found. Use 'snd add' instead.");
                std::process::exit(1);
            };
            srv.host = host.clone();
            srv.resolved = resolved;
            save_mutable_or_exit(&cfg, cli.local);
            println!("Updated: {alias} host -> {host}");
        }
        Some(Cmd::AddPath {
            server,
            path_alias,
            path,
        }) => {
            validate_alias(&path_alias, "path alias").unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let mut cfg = load_mutable_or_exit(cli.local);
            let Some(srv) = cfg.servers.get_mut(&server) else {
                eprintln!("Server '{server}' not found. Use 'snd add' to create it first.");
                std::process::exit(1);
            };
            if srv.paths.contains_key(&path_alias) {
                eprintln!(
                    "Path '{path_alias}' already exists on '{server}'. Use 'snd remove-path' first to replace it."
                );
                std::process::exit(1);
            }
            let path = path.replace("\\~", "~");
            srv.paths.insert(path_alias.clone(), path.clone());
            save_mutable_or_exit(&cfg, cli.local);
            println!("Added path: {server} {path_alias} -> {path}");
        }
        Some(Cmd::EditPath {
            server,
            path_alias,
            path,
        }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            let Some(srv) = cfg.servers.get_mut(&server) else {
                eprintln!("Server '{server}' not found.");
                std::process::exit(1);
            };
            if !srv.paths.contains_key(&path_alias) {
                eprintln!(
                    "Path '{path_alias}' not found on '{server}'. Use 'snd add-path' to create it."
                );
                std::process::exit(1);
            }
            let path = path.replace("\\~", "~");
            srv.paths.insert(path_alias.clone(), path.clone());
            save_mutable_or_exit(&cfg, cli.local);
            println!("Updated path: {server} {path_alias} -> {path}");
        }
        Some(Cmd::RemovePath { server, path_alias }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            let Some(srv) = cfg.servers.get_mut(&server) else {
                eprintln!("Server '{server}' not found.");
                std::process::exit(1);
            };
            if srv.paths.len() <= 1 {
                eprintln!(
                    "Cannot remove the only path on '{server}'. Use 'snd remove {server}' to delete the server."
                );
                std::process::exit(1);
            }
            if srv.paths.remove(&path_alias).is_none() {
                eprintln!("Path '{path_alias}' not found on '{server}'.");
                std::process::exit(1);
            }
            if srv.default == path_alias {
                srv.default = srv
                    .paths
                    .keys()
                    .next()
                    .cloned()
                    .expect("paths is non-empty");
                println!(
                    "Removed path: {server} {path_alias} (default reset to '{}')",
                    srv.default
                );
            } else {
                println!("Removed path: {server} {path_alias}");
            }
            save_mutable_or_exit(&cfg, cli.local);
        }
        Some(Cmd::SetDefault { server, path_alias }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            let Some(srv) = cfg.servers.get_mut(&server) else {
                eprintln!("Server '{server}' not found.");
                std::process::exit(1);
            };
            if !srv.paths.contains_key(&path_alias) {
                eprintln!("Path '{path_alias}' not found on '{server}'.");
                std::process::exit(1);
            }
            srv.default = path_alias.clone();
            save_mutable_or_exit(&cfg, cli.local);
            println!("Default path for {server}: {path_alias}");
        }
        Some(Cmd::List {
            target: Some(name),
            directory,
        }) => {
            let cfg = load_or_exit();
            let mut rest: Vec<String> = directory.into_iter().collect();
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &name, &mut rest, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    eprintln!("Run 'snd list' to see configured entries.");
                    std::process::exit(1);
                });
            if let Some(arg) = rest.first() {
                if cli.path.is_some() {
                    eprintln!("Cannot combine a path argument with -p/--path.");
                    std::process::exit(1);
                }
                let arg = arg.clone();
                for t in resolved.iter_mut() {
                    t.path = resolve_remote_arg(&t.path, &arg);
                }
            }
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_ls(resolved));
        }
        Some(Cmd::List { target: None, .. }) => {
            let cfg = load_or_exit();
            if cfg.servers.is_empty() && cfg.groups.is_empty() {
                println!("No servers configured. Use 'snd add <alias> <host> [path]' to add one.");
                return;
            }
            for (alias, srv) in &cfg.servers {
                let suffix = match check_ssh(srv) {
                    SshCheck::CachedMissing => "  (ssh: missing)",
                    SshCheck::CachedDrifted => "  (ssh: drift)",
                    _ => "",
                };
                println!("{alias}  [{}]{suffix}", srv.host);
                for (name, path) in &srv.paths {
                    let marker = if name == &srv.default { "*" } else { " " };
                    println!("  {marker} {name:<12}  {path}");
                }
            }
            if !cfg.groups.is_empty() {
                println!();
                println!("Groups:");
                for (name, g) in &cfg.groups {
                    if g.targets.is_empty() {
                        println!(
                            "{name}  (empty — add members with 'snd add-to-group {name} <target>')"
                        );
                        continue;
                    }
                    println!("{name}");
                    for t in &g.targets {
                        println!("    {t}");
                    }
                }
            }
        }
        Some(Cmd::AddGroup { name, targets }) => {
            validate_alias(&name, "group name").unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let mut cfg = load_mutable_or_exit(cli.local);
            let reference_cfg = if cli.local {
                load_or_exit()
            } else {
                cfg.clone()
            };
            if reference_cfg.servers.contains_key(&name) {
                eprintln!("'{name}' is already a server name.");
                std::process::exit(1);
            }
            if cfg.groups.contains_key(&name) {
                eprintln!(
                    "Group '{name}' already exists. Use 'snd add-to-group' or 'snd remove-group' first."
                );
                std::process::exit(1);
            }
            let mut resolved = Vec::with_capacity(targets.len());
            for t in &targets {
                match canonicalize_group_target(&reference_cfg, t) {
                    Ok(canonical) => {
                        if resolved.contains(&canonical) {
                            eprintln!("'{t}' is listed more than once.");
                            std::process::exit(1);
                        }
                        resolved.push(canonical);
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
            }
            cfg.groups.insert(
                name.clone(),
                Group {
                    targets: resolved.clone(),
                },
            );
            save_mutable_or_exit(&cfg, cli.local);
            println!("Added group: {name} -> {}", resolved.join(", "));
        }
        Some(Cmd::RemoveGroup { name }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            if cfg.groups.remove(&name).is_none() {
                eprintln!("Group '{name}' not found.");
                std::process::exit(1);
            }
            save_mutable_or_exit(&cfg, cli.local);
            println!("Removed group: {name}");
        }
        Some(Cmd::AddToGroup { group, target }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            let reference_cfg = if cli.local {
                load_or_exit()
            } else {
                cfg.clone()
            };
            let canonical = match canonicalize_group_target(&reference_cfg, &target) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            let Some(g) = cfg.groups.get_mut(&group) else {
                eprintln!(
                    "Group '{group}' not found. Use 'snd add-group {group} {target}' to create it."
                );
                std::process::exit(1);
            };
            if g.targets.iter().any(|t| t == &canonical) {
                eprintln!("'{canonical}' is already in group '{group}'.");
                std::process::exit(1);
            }
            g.targets.push(canonical.clone());
            save_mutable_or_exit(&cfg, cli.local);
            println!("Added '{canonical}' to group '{group}'");
        }
        Some(Cmd::RemoveFromGroup { group, target }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            let Some(g) = cfg.groups.get_mut(&group) else {
                eprintln!("Group '{group}' not found.");
                std::process::exit(1);
            };
            let before = g.targets.len();
            g.targets.retain(|t| t != &target);
            if g.targets.len() == before {
                eprintln!("'{target}' is not in group '{group}'.");
                std::process::exit(1);
            }
            if g.targets.is_empty() {
                cfg.groups.remove(&group);
                save_mutable_or_exit(&cfg, cli.local);
                println!("Removed '{target}' from group '{group}' (group now empty, deleted).");
            } else {
                save_mutable_or_exit(&cfg, cli.local);
                println!("Removed '{target}' from group '{group}'");
            }
        }
        Some(Cmd::Get {
            recursive,
            to,
            target,
            files,
        }) => {
            let cfg = load_or_exit();
            let mut files = files;
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if cli.path.is_none() {
                apply_positional_path_override(&mut resolved, &mut files, false);
            }
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            let code = dispatch_get(
                cli.force,
                cli.no_check,
                recursive,
                resolved,
                files,
                to.as_deref(),
                &options,
            );
            std::process::exit(code);
        }
        Some(Cmd::Delete {
            recursive,
            target,
            mut files,
        }) => {
            let cfg = load_or_exit();
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if cli.path.is_none() {
                apply_positional_path_override(&mut resolved, &mut files, false);
            }
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            let code = dispatch_delete(recursive, resolved, files, &options);
            std::process::exit(code);
        }
        Some(Cmd::Find {
            grep,
            regex,
            case_sensitive,
            depth,
            target,
            mut rest,
        }) => {
            let cfg = load_or_exit();
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &target, &mut rest, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    eprintln!("Run 'snd list' to see configured entries.");
                    std::process::exit(1);
                });
            if cli.path.is_none() {
                apply_positional_path_override(&mut resolved, &mut rest, false);
            }
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            let pattern = match rest.len() {
                0 => {
                    eprintln!(
                        "No search pattern given.\nUsage: snd find [-g] [-e] {target} [path-alias-or-dir] <pattern>"
                    );
                    std::process::exit(1);
                }
                1 => rest.remove(0),
                _ => {
                    eprintln!(
                        "Expected a single search pattern, got {}: {rest:?}\nQuote it if it contains spaces.",
                        rest.len()
                    );
                    std::process::exit(1);
                }
            };
            let code = dispatch_find(
                grep,
                regex,
                case_sensitive,
                depth,
                resolved,
                &pattern,
                options.json,
            );
            std::process::exit(code);
        }
        Some(Cmd::Cat { target, mut files }) => {
            let cfg = load_or_exit();
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    eprintln!("Run 'snd list' to see configured entries.");
                    std::process::exit(1);
                });
            if cli.path.is_none() {
                apply_positional_path_override(&mut resolved, &mut files, false);
            }
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_cat(resolved, files));
        }
        Some(Cmd::Diff {
            hash,
            target,
            mut files,
        }) => {
            let cfg = load_or_exit();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, cli.path.is_none())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if let Some(path) = cli.path.as_deref() {
                apply_path_override(&mut resolved, path);
            }
            if files.is_empty() {
                eprintln!("No local files specified for diff.");
                std::process::exit(1);
            }
            let (resolved, _) = expand_or_exit(resolved);
            expand_tildes(&mut files);
            std::process::exit(dispatch_diff(resolved, files, hash, &options));
        }
        Some(Cmd::Sync {
            delete,
            include,
            exclude,
            ignore_file,
            no_ignore,
            target,
            source,
        }) => {
            let cfg = load_or_exit();
            let mut no_args = Vec::new();
            let mut resolved = resolve_target_set(&cfg, &target, &mut no_args, false)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if let Some(path) = cli.path.as_deref() {
                apply_path_override(&mut resolved, path);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_sync(
                resolved,
                SyncRequest {
                    source,
                    delete,
                    force: cli.force,
                    include,
                    exclude,
                    ignore_file,
                    no_ignore,
                },
                &options,
            ));
        }
        Some(Cmd::Deploy {
            release,
            keep,
            target,
            mut files,
        }) => {
            let cfg = load_or_exit();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, cli.path.is_none())
                .unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(1);
                });
            if let Some(path) = cli.path.as_deref() {
                apply_path_override(&mut resolved, path);
            }
            let (resolved, _) = expand_or_exit(resolved);
            expand_tildes(&mut files);
            std::process::exit(dispatch_deploy(
                resolved,
                files,
                release.unwrap_or_else(generated_release_name),
                keep,
                &options,
            ));
        }
        Some(Cmd::Rollback {
            to,
            release,
            target,
            files,
        }) => {
            let cfg = load_or_exit();
            let mut args = Vec::new();
            let mut resolved =
                resolve_target_set(&cfg, &target, &mut args, false).unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(1);
                });
            if let Some(path) = cli.path.as_deref() {
                apply_path_override(&mut resolved, path);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_rollback(resolved, to, release, files, &options));
        }
        Some(Cmd::History { target, file }) => {
            let cfg = load_or_exit();
            let mut args = Vec::new();
            let mut resolved =
                resolve_target_set(&cfg, &target, &mut args, false).unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(1);
                });
            if let Some(path) = cli.path.as_deref() {
                apply_path_override(&mut resolved, path);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_history(resolved, file, &options));
        }
        Some(Cmd::Audit {
            file,
            last,
            command,
            failed,
        }) => {
            std::process::exit(commands::audit::run(
                Path::new(&file),
                last,
                command.as_deref(),
                failed,
                options.json,
            ));
        }
        Some(Cmd::Releases { target }) => {
            let cfg = load_or_exit();
            let mut args = Vec::new();
            let mut resolved =
                resolve_target_set(&cfg, &target, &mut args, false).unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(1);
                });
            if let Some(path) = cli.path.as_deref() {
                apply_path_override(&mut resolved, path);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_releases(resolved, &options));
        }
        Some(Cmd::Apply { manifest, name }) => {
            let path = PathBuf::from(&manifest);
            let manifest = Manifest::load(&path).unwrap_or_else(|error| {
                eprintln!("Manifest error: {error}");
                std::process::exit(1);
            });
            let selected = manifest.selected(name.as_deref()).unwrap_or_else(|error| {
                eprintln!("Manifest error: {error}");
                std::process::exit(1);
            });
            if options.json && selected.len() > 1 {
                eprintln!("Use --name with --json so apply emits one JSON document.");
                std::process::exit(1);
            }
            let cfg = load_or_exit();
            let mut worst = 0;
            for (deployment_name, deployment) in selected {
                let mut files = Manifest::resolved_files(&path, deployment);
                let mut no_args = Vec::new();
                let mut resolved =
                    resolve_target_set(&cfg, &deployment.target, &mut no_args, false)
                        .unwrap_or_else(|error| {
                            eprintln!("[{deployment_name}] {error}");
                            std::process::exit(1);
                        });
                if let Some(path) = deployment.path.as_deref() {
                    apply_path_override(&mut resolved, path);
                }
                if let Some(path) = cli.path.as_deref() {
                    apply_path_override(&mut resolved, path);
                }
                let (resolved, _) = expand_or_exit(resolved);
                expand_tildes(&mut files);
                let mut deployment_options = options.clone();
                deployment_options.atomic |= deployment.atomic;
                deployment_options.verify |= deployment.verify;
                deployment_options.resume |= deployment.resume;
                deployment_options.preserve |= deployment.preserve;
                deployment_options.compress |= deployment.compress;
                let code = if deployment.release {
                    dispatch_deploy(
                        resolved,
                        files,
                        deployment
                            .release_name
                            .clone()
                            .unwrap_or_else(generated_release_name),
                        deployment.keep,
                        &deployment_options,
                    )
                } else {
                    dispatch_send(
                        cli.force,
                        cli.no_check,
                        resolved,
                        files,
                        &deployment_options,
                    )
                };
                worst = worst.max(code);
                if code != 0 && options.fail_fast {
                    break;
                }
            }
            std::process::exit(worst);
        }
        Some(Cmd::Doctor { connect }) => {
            let cfg = load_or_exit();
            let reports: Vec<_> = cfg
                .servers
                .iter()
                .map(|(alias, server)| doctor_report(alias, server, connect, &options))
                .collect();
            if options.json {
                let failed = reports.iter().any(|report| {
                    report.config != "ok" && report.config != "not-applicable"
                        || report.connected == Some(false)
                });
                print_json("doctor", !failed, &reports);
                std::process::exit(if failed { 1 } else { 0 });
            }
            let mut had_issue = false;
            for (alias, srv) in &cfg.servers {
                match check_ssh(srv) {
                    SshCheck::CachedMissing => {
                        had_issue = true;
                        eprintln!(
                            "[{alias}] host '{}' is no longer a Host entry in ~/.ssh/config (was cached when added)",
                            srv.host
                        );
                    }
                    SshCheck::CachedDrifted => {
                        had_issue = true;
                        eprintln!(
                            "[{alias}] host '{}' resolves differently than when added — run 'snd refresh {alias}' to accept",
                            srv.host
                        );
                    }
                    SshCheck::NoCacheMatch => {
                        had_issue = true;
                        eprintln!(
                            "[{alias}] no resolution cache yet — run 'snd refresh {alias}' to capture current ssh config"
                        );
                    }
                    SshCheck::Match | SshCheck::NotApplicable | SshCheck::NoCacheNoMatch => {}
                }
            }
            if connect {
                for report in &reports {
                    if report.connected == Some(true) {
                        println!(
                            "[{}] connected to {} ({})",
                            report.alias,
                            report
                                .effective
                                .get("hostname")
                                .map(String::as_str)
                                .unwrap_or(&report.host),
                            report.message.as_deref().unwrap_or("path writable")
                        );
                    } else {
                        had_issue = true;
                        eprintln!(
                            "[{}] connection/path check failed: {}",
                            report.alias,
                            report.message.as_deref().unwrap_or("unknown error")
                        );
                    }
                }
            }
            if !had_issue {
                println!("All servers OK ({} checked).", cfg.servers.len());
            }
            std::process::exit(if had_issue { 1 } else { 0 });
        }
        Some(Cmd::Refresh { alias }) => {
            let mut cfg = load_mutable_or_exit(cli.local);
            let aliases: Vec<String> = match alias {
                Some(a) => {
                    if !cfg.servers.contains_key(&a) {
                        eprintln!("Server '{a}' not found.");
                        std::process::exit(1);
                    }
                    vec![a]
                }
                None => cfg.servers.keys().cloned().collect(),
            };
            let mut changed = 0;
            for a in &aliases {
                let Some(srv) = cfg.servers.get_mut(a) else {
                    continue;
                };
                let new = snd::ssh::lookup_alias(&srv.host).map(|h| SshResolved {
                    hostname: h.hostname,
                    user: h.user,
                });
                let old = srv.resolved.clone();
                if new != old {
                    srv.resolved = new.clone();
                    changed += 1;
                    let label = match (&old, &new) {
                        (None, Some(_)) => "captured",
                        (Some(_), None) => "cleared (alias not in ssh config)",
                        (Some(_), Some(_)) => "updated",
                        (None, None) => continue,
                    };
                    println!("[{a}] {label}");
                }
            }
            if changed > 0 {
                save_mutable_or_exit(&cfg, cli.local);
            } else {
                println!("Nothing to update ({} checked).", aliases.len());
            }
        }
        Some(Cmd::Init { force }) => {
            let path = std::env::current_dir()
                .unwrap_or_else(|e| {
                    eprintln!("Failed to determine current directory: {e}");
                    std::process::exit(1);
                })
                .join(".snd.toml");
            if path.exists() && !force {
                eprintln!(
                    "{} already exists (pass --force to replace it).",
                    path.display()
                );
                std::process::exit(1);
            }
            save_config_path(&Config::default(), &path, false).unwrap_or_else(|e| {
                eprintln!("Failed to write {}: {e}", path.display());
                std::process::exit(1);
            });
            println!("Created {}", path.display());
        }
        Some(Cmd::Config {
            action,
            resolved,
            paths,
        }) => {
            if paths {
                println!("global:  {}", config_path().display());
                println!(
                    "project: {}",
                    project_config_path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "(none)".to_string())
                );
                return;
            }
            match action.as_deref().unwrap_or("show") {
                "edit" => match commands::config::edit(cli.local) {
                    Ok(path) => println!("Validated {}", path.display()),
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                },
                "validate" => {
                    let cfg = load_or_exit();
                    let errors = validate_config(&cfg);
                    if options.json {
                        print_json("config-validate", errors.is_empty(), &errors);
                    } else if errors.is_empty() {
                        println!("Configuration is valid.");
                    } else {
                        eprintln!("Configuration has {} problem(s):", errors.len());
                        for error in &errors {
                            eprintln!("- {error}");
                        }
                    }
                    if !errors.is_empty() {
                        std::process::exit(1);
                    }
                }
                _ => {
                    let cfg = if cli.local && !resolved {
                        load_project_config_strict()
                            .map(|(_, config)| config)
                            .unwrap_or_else(|e| {
                                eprintln!("Config error: {e}");
                                std::process::exit(1);
                            })
                    } else {
                        load_or_exit()
                    };
                    if options.json {
                        print_json("config", true, &cfg);
                    } else {
                        print!(
                            "{}",
                            toml::to_string_pretty(&cfg).expect("serialize effective config")
                        );
                    }
                }
            }
        }
        Some(Cmd::Cache { action, older_than }) => {
            std::process::exit(commands::cache::run(&action, older_than, options.json));
        }
        Some(Cmd::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "snd", &mut io::stdout());
        }
        None => {
            let Some(name) = cli.server else {
                Cli::command().print_help().ok();
                std::process::exit(1);
            };

            let cfg = load_or_exit();
            let mut args = cli.args;
            let path_override = cli.path.as_deref();

            // For a group, alias positional is never consumed (each member has
            // its own path); for a server, the first arg may be a path alias —
            // but if `-p` was passed we skip alias parsing entirely.
            let mut targets = resolve_target_set(&cfg, &name, &mut args, path_override.is_none())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    eprintln!("Run 'snd list' to see configured entries.");
                    std::process::exit(1);
                });

            if let Some(p) = path_override {
                apply_path_override(&mut targets, p);
            } else {
                apply_positional_path_override(&mut targets, &mut args, true);
            }

            if args.is_empty() {
                eprintln!("No files specified.\nUsage: snd {name} [path-alias-or-dir] <file...>");
                std::process::exit(1);
            }

            let (targets, globbed) = expand_or_exit(targets);
            if globbed
                && !cli.force
                && !options.dry_run
                && !confirm_for_output(
                    &format!("Send to all {} resolved path(s)?", targets.len()),
                    options.json,
                )
            {
                eprintln!("Aborted.");
                std::process::exit(1);
            }

            expand_tildes(&mut args);

            let code = dispatch_send(cli.force, cli.no_check, targets, args, &options);
            std::process::exit(code);
        }
    }
}

fn expand_target_globs(
    targets: Vec<ResolvedTarget>,
) -> Result<(Vec<ResolvedTarget>, bool), String> {
    let mut out = Vec::new();
    let mut expanded_any = false;
    for t in targets {
        if !has_glob(&t.path) {
            out.push(t);
            continue;
        }
        expanded_any = true;
        let matches = expand_remote_glob(&t.host, &t.path)?;
        if matches.is_empty() {
            return Err(format!(
                "[{}] pattern '{}' matched no directories on {}",
                t.server_name, t.path, t.host
            ));
        }
        eprintln!(
            "[{}] {} — resolved to {} path(s) on {}:",
            t.server_name,
            t.path,
            matches.len(),
            t.host
        );
        for m in &matches {
            eprintln!("    {m}");
        }
        for m in matches {
            let label = glob_label(&t.path, &m);
            out.push(ResolvedTarget {
                server_name: format!("{}/{label}", t.server_name),
                path_name: t.path_name.clone(),
                host: t.host.clone(),
                path: m,
            });
        }
    }
    Ok((out, expanded_any))
}

fn expand_or_exit(targets: Vec<ResolvedTarget>) -> (Vec<ResolvedTarget>, bool) {
    expand_target_globs(targets).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    })
}

/// Replace every target's path with the override.
///
/// - `\~/foo` → `~/foo` (so a shell-escaped tilde reaches the remote literally)
/// - `./sub` or `./` → joined onto the target's existing path (per-target for
///   groups, so each member resolves under its own base)
/// - `../sub` → joined onto the existing path; the remote shell resolves `..`
/// - anything else (absolute, `~/...`, plain name) → used verbatim
fn apply_path_override(targets: &mut [ResolvedTarget], path: &str) {
    let p = path.replace("\\~", "~");
    for t in targets.iter_mut() {
        t.path = resolve_path_override(&t.path, &p);
    }
}

fn resolve_path_override(base: &str, override_: &str) -> String {
    if override_ == "." || override_ == "./" {
        return base.to_string();
    }
    if let Some(rest) = override_.strip_prefix("./") {
        return join_remote(base, rest);
    }
    if override_ == ".." || override_.starts_with("../") {
        return join_remote(base, override_);
    }
    override_.to_string()
}

fn apply_positional_path_override(
    targets: &mut [ResolvedTarget],
    args: &mut Vec<String>,
    local_files: bool,
) -> bool {
    if args.len() < 2 {
        return false;
    }
    let first = &args[0];
    let path_shaped = first.starts_with('/')
        || first.starts_with('~')
        || first == ".."
        || first.starts_with("../")
        || first.ends_with('/')
        || (!local_files && (first == "." || first.starts_with("./")));
    if !path_shaped {
        return false;
    }
    if local_files && !first.ends_with('/') && std::path::Path::new(first).exists() {
        return false;
    }
    let path = args.remove(0);
    apply_path_override(targets, &path);
    true
}

/// Resolve a server-or-group name. If `accept_path_alias_positional` is true
/// and the first arg matches a configured path-alias, it is consumed.
///
/// Fallback: if `name` matches no server or group but uniquely identifies a
/// path-alias under exactly one server, that server+path is used. Multiple
/// matches return an ambiguity error so the user can disambiguate explicitly.
fn resolve_target_set(
    cfg: &Config,
    name: &str,
    args: &mut Vec<String>,
    accept_path_alias_positional: bool,
) -> Result<Vec<ResolvedTarget>, String> {
    if let Some(g) = cfg.groups.get(name) {
        return resolve_group(g, &cfg.servers);
    }
    if let Some(srv) = cfg.servers.get(name) {
        let mut path_alias: Option<String> = None;
        if accept_path_alias_positional
            && let Some(first) = args.first()
            && srv.paths.contains_key(first)
        {
            path_alias = Some(first.clone());
            args.remove(0);
        }
        let target = resolve_server_target(name, &cfg.servers, path_alias.as_deref())?;
        return Ok(vec![target]);
    }
    let matches: Vec<&String> = cfg
        .servers
        .iter()
        .filter(|(_, s)| s.paths.contains_key(name))
        .map(|(alias, _)| alias)
        .collect();
    match matches.as_slice() {
        [] => Err(format!("Unknown server or group: {name}")),
        [server] => {
            let target = resolve_server_target(server, &cfg.servers, Some(name))?;
            Ok(vec![target])
        }
        many => {
            let options = many
                .iter()
                .map(|s| format!("'{s} {name}'"))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "Path alias '{name}' is ambiguous — matches multiple servers. Use one of: {options}"
            ))
        }
    }
}

fn format_local_age(meta: &std::fs::Metadata) -> String {
    let modified = match meta.modified() {
        Ok(t) => t,
        Err(_) => return "?".to_string(),
    };
    let elapsed = match modified.elapsed() {
        Ok(d) => d,
        Err(_) => return "future".to_string(),
    };
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn safe_download_suffix(path_name: &str) -> String {
    let suffix: String = path_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if suffix.is_empty() || suffix == "_" || suffix == "__" {
        "path".to_string()
    } else {
        suffix
    }
}

fn safe_download_label(label: &str) -> String {
    let safe = label
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .map(safe_download_suffix)
        .collect::<Vec<_>>()
        .join("/");
    if safe.is_empty() {
        "target".to_string()
    } else {
        safe
    }
}

fn download_local_labels(targets: &[ResolvedTarget]) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for target in targets {
        *counts.entry(target.server_name.clone()).or_default() += 1;
    }

    let mut used = BTreeSet::new();
    targets
        .iter()
        .map(|target| {
            let base = if counts
                .get(target.server_name.as_str())
                .is_some_and(|count| *count > 1)
            {
                format!(
                    "{}-{}",
                    safe_download_label(&target.server_name),
                    safe_download_suffix(&target.path_name)
                )
            } else {
                safe_download_label(&target.server_name)
            };
            if used.insert(base.clone()) {
                return base;
            }

            let mut occurrence = 2;
            loop {
                let candidate = format!("{base}-{occurrence}");
                if used.insert(candidate.clone()) {
                    return candidate;
                }
                occurrence += 1;
            }
        })
        .collect()
}

struct GetPlan {
    target: ResolvedTarget,
    local_dir: PathBuf,
    items: Vec<(String, PathBuf)>,
}

fn collision_key(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        raw.to_lowercase()
    } else {
        raw
    }
}

fn dispatch_get(
    cli_force: bool,
    no_check: bool,
    recursive: bool,
    targets: Vec<ResolvedTarget>,
    files: Vec<String>,
    to: Option<&str>,
    options: &TransferOptions,
) -> i32 {
    if options.atomic {
        eprintln!("--atomic currently applies to uploads only.");
        return 1;
    }
    let remote_files = files;
    if remote_files.is_empty() {
        eprintln!("No remote files specified.");
        return 1;
    }
    if let Some(file) = remote_files.iter().find(|file| has_unescaped_glob(file)) {
        eprintln!(
            "Wildcard downloads are not supported safely: '{file}'. Request explicit files so overwrite checks remain accurate; escape literal glob characters with a backslash."
        );
        return 1;
    }
    if recursive && (options.resume || options.verify) {
        eprintln!("--resume and --verify currently support regular-file downloads only.");
        return 1;
    }
    let remote_files: Vec<String> = remote_files
        .into_iter()
        .map(|file| unescape_glob_literals(&file))
        .collect();

    let multi = targets.len() > 1;
    let base = to.unwrap_or(".");

    let mut plans: Vec<GetPlan> = Vec::new();
    let local_labels = download_local_labels(&targets);
    for (target, local_label) in targets.into_iter().zip(local_labels) {
        let local_dir = if multi {
            std::path::PathBuf::from(base).join(local_label)
        } else {
            std::path::PathBuf::from(base)
        };
        let mut items = Vec::new();
        for f in &remote_files {
            let remote = resolve_remote_arg(&target.path, f);
            let basename = std::path::Path::new(remote.trim_end_matches('/'))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.clone());
            let local_file = local_dir.join(&basename);
            items.push((remote, local_file));
        }
        plans.push(GetPlan {
            target,
            local_dir,
            items,
        });
    }

    let mut destinations: BTreeMap<String, String> = BTreeMap::new();
    for plan in &plans {
        for (remote, local) in &plan.items {
            if let Some(previous) = destinations.insert(collision_key(local), remote.clone()) {
                eprintln!(
                    "Download collision: '{previous}' and '{remote}' both map to {}.",
                    local.display()
                );
                eprintln!("Choose separate destination directories or request one file at a time.");
                return 1;
            }
        }
    }

    if options.dry_run {
        let plan: Vec<_> = plans
            .iter()
            .flat_map(|plan| {
                plan.items.iter().map(|(remote, local)| {
                    serde_json::json!({
                        "action": "download",
                        "target": plan.target.server_name,
                        "host": plan.target.host,
                        "remote": remote,
                        "local": local,
                        "recursive": recursive,
                        "verify": options.verify,
                        "resume": options.resume,
                    })
                })
            })
            .collect();
        if options.json {
            print_json("get-plan", true, &plan);
        } else {
            for item in plan {
                println!(
                    "PLAN download {}:{} -> {}",
                    item["host"].as_str().unwrap_or("?"),
                    item["remote"].as_str().unwrap_or("?"),
                    item["local"].as_str().unwrap_or("?")
                );
            }
        }
        return 0;
    }

    if !no_check && !cli_force {
        let mut existing: Vec<std::path::PathBuf> = Vec::new();
        for plan in &plans {
            for (_, local) in &plan.items {
                if local.exists() {
                    existing.push(local.clone());
                }
            }
        }
        if !existing.is_empty() {
            if !options.json {
                println!("Local file(s) already exist:");
                for path in &existing {
                    match std::fs::metadata(path) {
                        Ok(meta) => {
                            let kind = if meta.is_dir() { " (dir)" } else { "" };
                            println!(
                                "  {:<40}  {:>10}  {}{}",
                                path.display(),
                                format_size(meta.len()),
                                format_local_age(&meta),
                                kind
                            );
                        }
                        Err(_) => println!("  {}", path.display()),
                    }
                }
            }
            if !confirm_for_output("Overwrite local files?", options.json) {
                eprintln!("Aborted.");
                return 1;
            }
        }
    }

    for plan in &plans {
        if let Err(e) = std::fs::create_dir_all(&plan.local_dir) {
            eprintln!("Failed to create {}: {e}", plan.local_dir.display());
            return 1;
        }
    }

    let worker_options = options.clone();
    let results = run_parallel(plans, options.jobs, options.fail_fast, move |plan| {
        download_plan(plan, recursive, &worker_options)
    });
    print_results("get", &results, options)
}

fn download_plan(plan: GetPlan, recursive: bool, options: &TransferOptions) -> OperationResult {
    let label = plan.target.server_name.clone();
    if options.resume && recursive {
        return OperationResult::failure(
            label,
            "download",
            0,
            "--resume currently supports regular files only",
        );
    }

    let mut attempts = 0;
    if options.resume {
        for (remote, local) in &plan.items {
            let lock = match acquire_local_lock(local) {
                Ok(lock) => lock,
                Err(e) => return OperationResult::failure(label, "lock", attempts, e),
            };
            if let Err(e) = validate_download_resume(&plan.target.host, remote, local, options) {
                release_local_lock(&lock);
                return OperationResult::failure(label, "resume-check", attempts, e);
            }
            if options.progress && !options.json {
                eprintln!("[{}] downloading {remote}", plan.target.server_name);
            }
            let (used, status) = retry_status(options.retries, || {
                sftp_reget(&plan.target.host, remote, local, options)
            });
            release_local_lock(&lock);
            attempts += used;
            match status {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    return OperationResult::failure(
                        label,
                        "download",
                        attempts,
                        format!("sftp exited with {}", status.code().unwrap_or(1)),
                    );
                }
                Err(e) => {
                    return OperationResult::failure(label, "download", attempts, e.to_string());
                }
            }
        }
    } else {
        let display_sources: Vec<String> = plan
            .items
            .iter()
            .map(|(remote, _)| format!("{}:{}", plan.target.host, remote))
            .collect();
        let sources: Vec<String> = plan
            .items
            .iter()
            .map(|(remote, _)| format!("{}:{}", plan.target.host, scp_literal_remote_path(remote)))
            .collect();
        if options.progress && !options.json {
            eprintln!("[{}] download started", plan.target.server_name);
        }
        if !options.json {
            println!(
                "scp {} -> {}",
                display_sources.join(" "),
                plan.local_dir.display()
            );
        }
        let (used, status) = retry_status(options.retries, || {
            let mut cmd = Command::new("scp");
            options.apply_scp(&mut cmd);
            if options.json {
                cmd.stdout(Stdio::null());
            }
            if recursive {
                cmd.arg("-r");
            }
            cmd.arg("--").args(&sources).arg(&plan.local_dir).status()
        });
        attempts = used;
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => {
                return OperationResult::failure(
                    label,
                    "download",
                    attempts,
                    format!("scp exited with {}", status.code().unwrap_or(1)),
                );
            }
            Err(e) => return OperationResult::failure(label, "download", attempts, e.to_string()),
        }
    }

    if options.verify {
        if recursive {
            return OperationResult::failure(
                label,
                "verify",
                attempts,
                "--verify currently supports regular files only",
            );
        }
        for (remote, local) in &plan.items {
            let local_hash = match local_sha256(local) {
                Ok(hash) => hash,
                Err(e) => return OperationResult::failure(label, "verify", attempts, e),
            };
            match remote_sha256(&plan.target.host, remote, options) {
                Ok(hash) if hash == local_hash => {}
                Ok(_) => {
                    return OperationResult::failure(
                        label,
                        "verify",
                        attempts,
                        format!("checksum mismatch for {}", local.display()),
                    );
                }
                Err(e) => return OperationResult::failure(label, "verify", attempts, e),
            }
        }
    }
    let result = OperationResult::success(label, "download", attempts.max(1));
    if recursive {
        result
    } else {
        let bytes = plan
            .items
            .iter()
            .filter_map(|(_, local)| std::fs::metadata(local).ok())
            .map(|metadata| metadata.len())
            .sum();
        result.with_bytes(bytes)
    }
}
