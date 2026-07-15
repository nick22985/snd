use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::remote::join_remote;
use crate::transfer::{
    TransferOptions, acquire_remote_lock, release_remote_lock, remote_shell_path, shell_quote,
};

static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct SendBackup {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendRollback {
    Restored(String),
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendHistoryFile {
    pub name: String,
    pub previous_state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendHistoryEntry {
    pub id: String,
    pub timestamp_ms: Option<u128>,
    pub storage: String,
    pub files: Vec<SendHistoryFile>,
}

#[derive(Debug, Clone)]
struct BackupItem {
    transaction: String,
    index: String,
    name: String,
    legacy: bool,
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

fn output_error(output: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr.trim();
    if message.is_empty() {
        fallback.to_string()
    } else {
        message.to_string()
    }
}

fn checked_output(host: &str, remote: &str, options: &TransferOptions) -> Result<String, String> {
    let output = ssh_output(host, remote, options)?;
    if !output.status.success() {
        return Err(output_error(&output, "remote backup command failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn backup_root(base: &str) -> String {
    join_remote(&state_root(base), "backups")
}

fn legacy_backup_root(base: &str) -> String {
    join_remote(base, ".snd/backups")
}

fn state_root(base: &str) -> String {
    let key = format!("{:x}", Sha256::digest(base.as_bytes()));
    format!("~/.local/share/snd/targets/{key}")
}

fn backup_roots(base: &str) -> [String; 2] {
    [backup_root(base), legacy_backup_root(base)]
}

fn backup_directory(base: &str, id: &str) -> String {
    join_remote(&backup_root(base), id)
}

fn generated_backup_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:020}-{}-{sequence}", std::process::id())
}

pub fn acquire_send_lock(
    host: &str,
    base: &str,
    options: &TransferOptions,
) -> Result<String, String> {
    let state = state_root(base);
    checked_output(
        host,
        &format!("mkdir -p -- {}", remote_shell_path(&state)),
        options,
    )?;
    acquire_remote_lock(host, &join_remote(&state, "send"), options)
}

pub fn release_send_lock(host: &str, lock: &str, options: &TransferOptions) {
    release_remote_lock(host, lock, options);
}

pub fn prepare_send_backup(
    host: &str,
    base: &str,
    names: &[String],
    options: &TransferOptions,
) -> Result<SendBackup, String> {
    let id = generated_backup_id();
    let directory = backup_directory(base, &id);
    let items = join_remote(&directory, "items");
    let root = backup_root(base);
    let legacy_root = legacy_backup_root(base);
    let legacy_state = join_remote(base, ".snd");
    let mut command = format!(
        "set -e; new_root={root}; legacy_root={legacy_root}; mkdir -p -- \"$new_root\"; \
         if [ -d \"$legacy_root\" ]; then for old in \"$legacy_root\"/*; do \
         if [ -d \"$old\" ]; then migrated=\"$new_root/${{old##*/}}\"; \
         if [ ! -e \"$migrated\" ]; then mv -- \"$old\" \"$migrated\"; fi; fi; done; \
         rmdir -- \"$legacy_root\" 2>/dev/null || true; rmdir -- {legacy_state} 2>/dev/null || true; fi; \
         for stale in \"$new_root\"/*; do \
         if [ -d \"$stale\" ] && [ ! -f \"$stale/.snd-complete\" ]; then rm -rf -- \"$stale\"; fi; done; \
         mkdir -p -- {items}",
        root = remote_shell_path(&root),
        legacy_root = remote_shell_path(&legacy_root),
        legacy_state = remote_shell_path(&legacy_state),
        items = remote_shell_path(&items),
    );

    for (index, name) in names.iter().enumerate() {
        if name.is_empty()
            || matches!(name.as_str(), "." | ".." | ".snd")
            || name.contains(['/', '\n', '\r'])
        {
            return Err(format!(
                "cannot create rollback backup for filename '{name}'"
            ));
        }
        let item = join_remote(&items, &index.to_string());
        let destination = join_remote(base, name);
        let original = join_remote(&item, "original");
        let name_file = join_remote(&item, "name");
        let state_file = join_remote(&item, "state");
        command.push_str(&format!(
            "; mkdir -- {item}; printf '%s\n' {name} > {name_file}; \
             if [ -e {destination} ] || [ -L {destination} ]; then \
             printf 'present\n' > {state_file}; cp -a -- {destination} {original}; \
             else printf 'missing\n' > {state_file}; fi",
            item = remote_shell_path(&item),
            name = shell_quote(name),
            name_file = remote_shell_path(&name_file),
            destination = remote_shell_path(&destination),
            state_file = remote_shell_path(&state_file),
            original = remote_shell_path(&original),
        ));
    }

    if let Err(error) = checked_output(host, &command, options) {
        let _ = checked_output(
            host,
            &format!("rm -rf -- {}", remote_shell_path(&directory)),
            options,
        );
        return Err(error);
    }
    Ok(SendBackup { id })
}

fn restore_command(base: &str, directory_expression: &str, remove: bool) -> String {
    let base = remote_shell_path(base);
    let remove_command = if remove {
        "rm -rf -- \"$transaction\";"
    } else {
        ""
    };
    format!(
        "set -e; transaction={directory_expression}; \
         for item in \"$transaction\"/items/*; do \
         [ -d \"$item\" ] || continue; IFS= read -r name < \"$item/name\"; \
         case \"$name\" in ''|.|..|.snd|*/*) echo 'invalid send backup entry' >&2; exit 5;; esac; \
         destination={base}/\"$name\"; IFS= read -r state < \"$item/state\"; \
         case \"$state\" in \
         present) rm -rf -- \"$destination\"; cp -a -- \"$item/original\" \"$destination\";; \
         missing) rm -rf -- \"$destination\";; \
         *) echo 'invalid send backup state' >&2; exit 5;; esac; done; \
         id=${{transaction##*/}}; {remove_command} printf '%s\n' \"$id\""
    )
}

pub fn restore_send_backup(
    host: &str,
    base: &str,
    backup: &SendBackup,
    options: &TransferOptions,
) -> Result<(), String> {
    let directory = backup_directory(base, &backup.id);
    let command = restore_command(base, &remote_shell_path(&directory), true);
    checked_output(host, &command, options).map(|_| ())
}

pub fn complete_send_backup(
    host: &str,
    base: &str,
    backup: &SendBackup,
    keep: usize,
    options: &TransferOptions,
) -> Result<(), String> {
    let directory = backup_directory(base, &backup.id);
    let marker = join_remote(&directory, ".snd-complete");
    checked_output(
        host,
        &format!(": > {}", remote_shell_path(&marker)),
        options,
    )?;
    prune_send_backups(host, base, keep, options)
}

pub fn rollback_latest_send(
    host: &str,
    base: &str,
    options: &TransferOptions,
) -> Result<SendRollback, String> {
    let [root, legacy_root] = backup_roots(base).map(|root| remote_shell_path(&root));
    let selection = format!(
        "transaction=; for root in {root} {legacy_root}; do if [ -d \"$root\" ]; then \
         for candidate in \"$root\"/*; do if [ -d \"$candidate\" ] && [ -f \"$candidate/.snd-complete\" ]; then \
         if [ -z \"$transaction\" ] || [ \"${{candidate##*/}}\" \\> \"${{transaction##*/}}\" ]; then \
         transaction=$candidate; fi; fi; done; fi; done; [ -n \"$transaction\" ] || exit 4"
    );
    let command = restore_command(
        base,
        &format!("$({selection}; printf '%s' \"$transaction\")"),
        true,
    );
    let output = ssh_output(host, &command, options)?;
    if output.status.code() == Some(4) {
        return Ok(SendRollback::None);
    }
    if !output.status.success() {
        return Err(output_error(&output, "direct-send rollback failed"));
    }
    Ok(SendRollback::Restored(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn find_latest_backup_item(
    host: &str,
    base: &str,
    name: &str,
    options: &TransferOptions,
) -> Result<BackupItem, String> {
    let [root, legacy_root] = backup_roots(base).map(|root| remote_shell_path(&root));
    let command = format!(
        "set -e; selected=; transaction=; selected_kind=; for root in {root} {legacy_root}; do \
         if [ \"$root\" = {root} ]; then root_kind=new; else root_kind=legacy; fi; if [ -d \"$root\" ]; then \
         for candidate in \"$root\"/*; do if [ -d \"$candidate\" ] && [ -f \"$candidate/.snd-complete\" ]; then \
         for item in \"$candidate\"/items/*; do if [ -d \"$item\" ]; then \
         IFS= read -r stored < \"$item/name\"; if [ \"$stored\" = {name} ]; then \
         if [ -z \"$transaction\" ] || [ \"${{candidate##*/}}\" \\> \"${{transaction##*/}}\" ]; then \
         transaction=$candidate; selected=$item; selected_kind=$root_kind; fi; fi; fi; done; fi; done; fi; done; \
         [ -n \"$selected\" ] || {{ echo 'no rollback snapshot for {display}' >&2; exit 4; }}; \
         printf '%s\t%s\t%s\n' \"${{transaction##*/}}\" \"${{selected##*/}}\" \"$selected_kind\"",
        name = shell_quote(name),
        display = name.replace('\'', "'\\''"),
    );
    let output = ssh_output(host, &command, options)?;
    if !output.status.success() {
        return Err(output_error(
            &output,
            &format!("no rollback snapshot for {name}"),
        ));
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let mut fields = value.trim().split('\t');
    let transaction = fields
        .next()
        .ok_or_else(|| "invalid rollback snapshot selection".to_string())?;
    let index = fields
        .next()
        .ok_or_else(|| "invalid rollback snapshot selection".to_string())?;
    let kind = fields
        .next()
        .ok_or_else(|| "invalid rollback snapshot selection".to_string())?;
    if fields.next().is_some() {
        return Err("invalid rollback snapshot selection".to_string());
    }
    if transaction.is_empty()
        || !transaction
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        || index.is_empty()
        || !index.chars().all(|character| character.is_ascii_digit())
        || !matches!(kind, "new" | "legacy")
    {
        return Err("invalid rollback snapshot selection".to_string());
    }
    Ok(BackupItem {
        transaction: transaction.to_string(),
        index: index.to_string(),
        name: name.to_string(),
        legacy: kind == "legacy",
    })
}

fn restore_backup_item(
    host: &str,
    base: &str,
    item: &BackupItem,
    options: &TransferOptions,
) -> Result<(), String> {
    let transaction = if item.legacy {
        join_remote(&legacy_backup_root(base), &item.transaction)
    } else {
        backup_directory(base, &item.transaction)
    };
    let item_directory = join_remote(&join_remote(&transaction, "items"), &item.index);
    let destination = join_remote(base, &item.name);
    let command = format!(
        "set -e; item={item}; transaction={transaction}; \
         [ -d \"$item\" ] && [ -f \"$transaction/.snd-complete\" ]; \
         IFS= read -r stored < \"$item/name\"; [ \"$stored\" = {name} ] || \
         {{ echo 'rollback snapshot changed during restore' >&2; exit 5; }}; \
         IFS= read -r state < \"$item/state\"; case \"$state\" in \
         present) rm -rf -- {destination}; cp -a -- \"$item/original\" {destination};; \
         missing) rm -rf -- {destination};; \
         *) echo 'invalid send backup state' >&2; exit 5;; esac; \
         rm -rf -- \"$item\"; remaining=; for entry in \"$transaction\"/items/*; do \
         if [ -d \"$entry\" ]; then remaining=1; break; fi; done; \
         if [ -z \"$remaining\" ]; then rm -rf -- \"$transaction\"; fi",
        item = remote_shell_path(&item_directory),
        transaction = remote_shell_path(&transaction),
        name = shell_quote(&item.name),
        destination = remote_shell_path(&destination),
    );
    checked_output(host, &command, options).map(|_| ())
}

pub fn rollback_named_sends(
    host: &str,
    base: &str,
    names: &[String],
    options: &TransferOptions,
) -> Result<Vec<(String, String)>, String> {
    let items = names
        .iter()
        .map(|name| find_latest_backup_item(host, base, name, options))
        .collect::<Result<Vec<_>, _>>()?;
    let mut restored = Vec::with_capacity(items.len());
    for item in items {
        restore_backup_item(host, base, &item, options)?;
        restored.push((item.name, item.transaction));
    }
    Ok(restored)
}

pub fn send_history(
    host: &str,
    base: &str,
    filter: Option<&str>,
    options: &TransferOptions,
) -> Result<Vec<SendHistoryEntry>, String> {
    let [root, legacy_root] = backup_roots(base).map(|root| remote_shell_path(&root));
    let command = format!(
        "set -e; for root in {root} {legacy_root}; do \
         if [ \"$root\" = {root} ]; then storage=remote-home; else storage=legacy-target; fi; \
         if [ -d \"$root\" ]; then for transaction in \"$root\"/*; do \
         if [ -d \"$transaction\" ] && [ -f \"$transaction/.snd-complete\" ]; then \
         for item in \"$transaction\"/items/*; do if [ -d \"$item\" ]; then \
         IFS= read -r name < \"$item/name\"; IFS= read -r state < \"$item/state\"; \
         printf '%s\\0%s\\0%s\\0%s\\0' \"${{transaction##*/}}\" \"$storage\" \"$name\" \"$state\"; \
         fi; done; fi; done; fi; done"
    );
    let output = ssh_output(host, &command, options)?;
    if !output.status.success() {
        return Err(output_error(&output, "failed to read send history"));
    }
    let fields: Vec<&[u8]> = output.stdout.split(|byte| *byte == 0).collect();
    if !fields.last().is_some_and(|field| field.is_empty()) {
        return Err("invalid send history response".to_string());
    }
    let usable = fields.len().saturating_sub(1);
    if !usable.is_multiple_of(4) {
        return Err("invalid send history response".to_string());
    }
    let mut grouped: std::collections::BTreeMap<(String, String), Vec<SendHistoryFile>> =
        std::collections::BTreeMap::new();
    for record in fields[..usable].chunks_exact(4) {
        let id = String::from_utf8_lossy(record[0]).into_owned();
        let storage = String::from_utf8_lossy(record[1]).into_owned();
        let name = String::from_utf8_lossy(record[2]).into_owned();
        let previous_state = String::from_utf8_lossy(record[3]).into_owned();
        if !matches!(previous_state.as_str(), "present" | "missing")
            || !matches!(storage.as_str(), "remote-home" | "legacy-target")
        {
            return Err("invalid send history response".to_string());
        }
        if filter.is_none_or(|wanted| wanted == name) {
            grouped
                .entry((id, storage))
                .or_default()
                .push(SendHistoryFile {
                    name,
                    previous_state,
                });
        }
    }
    let mut history: Vec<_> = grouped
        .into_iter()
        .map(|((id, storage), files)| {
            let timestamp_ms = id
                .split('-')
                .next()
                .and_then(|nanos| nanos.parse::<u128>().ok())
                .map(|nanos| nanos / 1_000_000);
            SendHistoryEntry {
                id,
                timestamp_ms,
                storage,
                files,
            }
        })
        .collect();
    history.sort_by(|left, right| right.id.cmp(&left.id));
    Ok(history)
}

pub fn prune_send_backups(
    host: &str,
    base: &str,
    keep: usize,
    options: &TransferOptions,
) -> Result<(), String> {
    if keep == 0 {
        return Ok(());
    }
    let root = remote_shell_path(&backup_root(base));
    let command = format!(
        "set -e; count=0; if [ -d {root} ]; then for transaction in {root}/*; do \
         [ -d \"$transaction\" ] && [ -f \"$transaction/.snd-complete\" ] && count=$((count + 1)); done; \
         while [ \"$count\" -gt {keep} ]; do oldest=; for transaction in {root}/*; do \
         if [ -d \"$transaction\" ] && [ -f \"$transaction/.snd-complete\" ]; then oldest=$transaction; break; fi; done; \
         [ -n \"$oldest\" ] || break; rm -rf -- \"$oldest\"; count=$((count - 1)); done; fi"
    );
    checked_output(host, &command, options).map(|_| ())
}
