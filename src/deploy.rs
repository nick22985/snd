use serde::Serialize;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::remote::join_remote;
use crate::transfer::{
    TransferOptions, acquire_remote_lock, release_remote_lock, remote_shell_path, shell_quote,
};

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseState {
    pub active: Option<String>,
    pub previous: Option<String>,
    pub releases: Vec<String>,
}

pub fn generated_release_name() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{seconds}-{}", std::process::id())
}

pub fn validate_release_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(format!(
            "invalid release name '{name}'; use letters, numbers, '-' and '_'"
        ));
    }
    Ok(())
}

pub fn release_directory(base: &str, release: &str) -> String {
    join_remote(&join_remote(base, ".snd/releases"), release)
}

pub fn acquire_deploy_lock(
    host: &str,
    base: &str,
    options: &TransferOptions,
) -> Result<String, String> {
    let state = join_remote(base, ".snd");
    checked_output(
        host,
        &format!("mkdir -p -- {}", remote_shell_path(&state)),
        options,
    )?;
    let resource = join_remote(base, ".snd/deploy");
    acquire_remote_lock(host, &resource, options)
}

pub fn release_deploy_lock(host: &str, lock: &str, options: &TransferOptions) {
    release_remote_lock(host, lock, options);
}

fn ssh_output(
    host: &str,
    remote: &str,
    options: &TransferOptions,
) -> Result<std::process::Output, String> {
    let mut command = Command::new("ssh");
    options.apply_ssh(&mut command);
    command
        .arg("--")
        .arg(host)
        .arg(remote)
        .output()
        .map_err(|error| format!("ssh: {error}"))
}

fn checked_output(host: &str, remote: &str, options: &TransferOptions) -> Result<String, String> {
    let output = ssh_output(host, remote, options)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            format!(
                "remote command exited with {}",
                output.status.code().unwrap_or(1)
            )
        } else {
            stderr.trim().to_string()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn prepare_release(
    host: &str,
    base: &str,
    release: &str,
    options: &TransferOptions,
) -> Result<String, String> {
    validate_release_name(release)?;
    let directory = release_directory(base, release);
    let releases = join_remote(base, ".snd/releases");
    let command = if options.resume {
        let marker = join_remote(&directory, ".snd-complete");
        format!(
            "test ! -f {}; mkdir -p -- {}",
            remote_shell_path(&marker),
            remote_shell_path(&directory)
        )
    } else {
        format!(
            "mkdir -p -- {}; mkdir -- {}",
            remote_shell_path(&releases),
            remote_shell_path(&directory)
        )
    };
    checked_output(host, &command, options)?;
    Ok(directory)
}

pub fn complete_release(
    host: &str,
    base: &str,
    release: &str,
    options: &TransferOptions,
) -> Result<(), String> {
    let marker = join_remote(&release_directory(base, release), ".snd-complete");
    checked_output(
        host,
        &format!(": > {}", remote_shell_path(&marker)),
        options,
    )?;
    Ok(())
}

pub fn remove_release(host: &str, base: &str, release: &str, options: &TransferOptions) {
    let directory = release_directory(base, release);
    let _ = checked_output(
        host,
        &format!("rm -rf -- {}", remote_shell_path(&directory)),
        options,
    );
}

pub fn activate_release(
    host: &str,
    base: &str,
    release: &str,
    options: &TransferOptions,
) -> Result<(), String> {
    validate_release_name(release)?;
    let state = join_remote(base, ".snd");
    let releases = join_remote(&state, "releases");
    let current = join_remote(&state, "current");
    let previous = join_remote(&state, "previous");
    let current_temp = join_remote(&state, ".current.tmp");
    let previous_temp = join_remote(&state, ".previous.tmp");
    let release_directory = join_remote(&releases, release);
    let completion_marker = join_remote(&release_directory, ".snd-complete");
    let target = format!("releases/{release}");
    let command = format!(
        "set -e; mkdir -p -- {state}; test -d {release_directory}; \
         test -f {completion_marker}; old=$(readlink {current} 2>/dev/null || true); \
         if [ -n \"$old\" ]; then printf '%s\\n' \"$old\" > {previous_temp}; mv -f -- {previous_temp} {previous}; else rm -f -- {previous}; fi; \
         rm -f -- {current_temp}; ln -s -- {target} {current_temp}; \
         if mv -Tf -- {current_temp} {current} 2>/dev/null; then :; else rm -f -- {current}; mv -f -- {current_temp} {current}; fi",
        state = remote_shell_path(&state),
        release_directory = remote_shell_path(&release_directory),
        completion_marker = remote_shell_path(&completion_marker),
        current = remote_shell_path(&current),
        previous_temp = remote_shell_path(&previous_temp),
        previous = remote_shell_path(&previous),
        current_temp = remote_shell_path(&current_temp),
        target = shell_quote(&target),
    );
    checked_output(host, &command, options)?;
    Ok(())
}

pub fn rollback_release(
    host: &str,
    base: &str,
    requested: Option<&str>,
    options: &TransferOptions,
) -> Result<String, String> {
    if let Some(release) = requested {
        validate_release_name(release)?;
    }
    let state = join_remote(base, ".snd");
    let current = join_remote(&state, "current");
    let previous = join_remote(&state, "previous");
    let current_temp = join_remote(&state, ".current.tmp");
    let previous_temp = join_remote(&state, ".previous.tmp");
    let target_setup = match requested {
        Some(release) => format!("target={}", shell_quote(&format!("releases/{release}"))),
        None => format!(
            "target=$(cat {} 2>/dev/null || true)",
            remote_shell_path(&previous)
        ),
    };
    let command = format!(
        "set -e; {target_setup}; case \"$target\" in releases/*) name=${{target#releases/}};; *) echo 'no previous release recorded' >&2; exit 4;; esac; \
         case \"$name\" in ''|*[!A-Za-z0-9_-]*) echo 'invalid previous release record' >&2; exit 4;; esac; \
         test -d {state}/\"$target\"; test -f {state}/\"$target\"/.snd-complete; old=$(readlink {current} 2>/dev/null || true); \
         rm -f -- {current_temp}; ln -s -- \"$target\" {current_temp}; \
         if mv -Tf -- {current_temp} {current} 2>/dev/null; then :; else rm -f -- {current}; mv -f -- {current_temp} {current}; fi; \
         if [ -n \"$old\" ]; then printf '%s\\n' \"$old\" > {previous_temp}; mv -f -- {previous_temp} {previous}; fi; \
         printf '%s\\n' \"$name\"",
        state = remote_shell_path(&state),
        current = remote_shell_path(&current),
        current_temp = remote_shell_path(&current_temp),
        previous_temp = remote_shell_path(&previous_temp),
        previous = remote_shell_path(&previous),
    );
    let output = checked_output(host, &command, options)?;
    Ok(output.trim().to_string())
}

pub fn release_state(
    host: &str,
    base: &str,
    options: &TransferOptions,
) -> Result<ReleaseState, String> {
    let state = join_remote(base, ".snd");
    let current = join_remote(&state, "current");
    let previous = join_remote(&state, "previous");
    let releases = join_remote(&state, "releases");
    let command = format!(
        "printf 'active\\t'; readlink {current} 2>/dev/null || true; \
         printf 'previous\\t'; cat {previous} 2>/dev/null || true; \
         if [ -d {releases} ]; then for path in {releases}/*; do [ -d \"$path\" ] && [ -f \"$path/.snd-complete\" ] && printf 'release\\t%s\\n' \"${{path##*/}}\"; done; fi",
        current = remote_shell_path(&current),
        previous = remote_shell_path(&previous),
        releases = remote_shell_path(&releases),
    );
    let output = checked_output(host, &command, options)?;
    let mut active = None;
    let mut previous_release = None;
    let mut releases_list = Vec::new();
    for line in output.lines() {
        let Some((kind, value)) = line.split_once('\t') else {
            continue;
        };
        let value = value.strip_prefix("releases/").unwrap_or(value);
        if value.is_empty() {
            continue;
        }
        match kind {
            "active" => active = Some(value.to_string()),
            "previous" => previous_release = Some(value.to_string()),
            "release" => releases_list.push(value.to_string()),
            _ => {}
        }
    }
    releases_list.sort();
    Ok(ReleaseState {
        active,
        previous: previous_release,
        releases: releases_list,
    })
}

pub fn prune_releases(
    host: &str,
    base: &str,
    keep: usize,
    options: &TransferOptions,
) -> Result<(), String> {
    if keep == 0 {
        return Ok(());
    }
    let state = release_state(host, base, options)?;
    if state.releases.len() <= keep {
        return Ok(());
    }
    let protected = [state.active.as_deref(), state.previous.as_deref()];
    let remove_count = state.releases.len() - keep;
    for release in state.releases.iter().take(remove_count) {
        if protected.contains(&Some(release.as_str())) {
            continue;
        }
        let directory = release_directory(base, release);
        let command = format!("rm -rf -- {}", remote_shell_path(&directory));
        checked_output(host, &command, options)?;
    }
    Ok(())
}
