use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Default)]
pub struct TransferOptions {
    pub dry_run: bool,
    pub json: bool,
    pub jobs: usize,
    pub fail_fast: bool,
    pub retries: u32,
    pub preserve: bool,
    pub compress: bool,
    pub limit: Option<u64>,
    pub identity: Option<String>,
    pub ssh_config: Option<String>,
    pub atomic: bool,
    pub verify: bool,
    pub resume: bool,
    pub progress: bool,
    pub audit_log: Option<PathBuf>,
    pub backup: bool,
    pub backup_keep: usize,
}

impl TransferOptions {
    pub fn apply_scp(&self, cmd: &mut Command) {
        if self.preserve {
            cmd.arg("-p");
        }
        if self.compress {
            cmd.arg("-C");
        }
        if let Some(limit) = self.limit {
            cmd.arg("-l").arg(limit.to_string());
        }
        if let Some(identity) = &self.identity {
            cmd.arg("-i").arg(identity);
        }
        if let Some(config) = &self.ssh_config {
            cmd.arg("-F").arg(config);
        }
    }

    pub fn apply_ssh(&self, cmd: &mut Command) {
        if self.compress {
            cmd.arg("-C");
        }
        if let Some(identity) = &self.identity {
            cmd.arg("-i").arg(identity);
        }
        if let Some(config) = &self.ssh_config {
            cmd.arg("-F").arg(config);
        }
    }

    fn apply_sftp(&self, cmd: &mut Command) {
        if self.compress {
            cmd.arg("-C");
        }
        if let Some(limit) = self.limit {
            cmd.arg("-l").arg(limit.to_string());
        }
        if let Some(identity) = &self.identity {
            cmd.arg("-i").arg(identity);
        }
        if let Some(config) = &self.ssh_config {
            cmd.arg("-F").arg(config);
        }
    }

    pub fn rsync_shell(&self) -> String {
        let mut parts = vec!["ssh".to_string()];
        if self.compress {
            parts.push("-C".to_string());
        }
        if let Some(identity) = &self.identity {
            parts.push("-i".to_string());
            parts.push(shell_quote(identity));
        }
        if let Some(config) = &self.ssh_config {
            parts.push("-F".to_string());
            parts.push(shell_quote(config));
        }
        parts.join(" ")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OperationResult {
    pub target: String,
    pub action: String,
    pub success: bool,
    pub attempts: u32,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl OperationResult {
    pub fn success(target: impl Into<String>, action: impl Into<String>, attempts: u32) -> Self {
        Self {
            target: target.into(),
            action: action.into(),
            success: true,
            attempts,
            duration_ms: 0,
            bytes: None,
            message: None,
        }
    }

    pub fn failure(
        target: impl Into<String>,
        action: impl Into<String>,
        attempts: u32,
        message: impl Into<String>,
    ) -> Self {
        Self {
            target: target.into(),
            action: action.into(),
            success: false,
            attempts,
            duration_ms: 0,
            bytes: None,
            message: Some(message.into()),
        }
    }

    pub fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }
}

pub fn run_parallel<T, F>(
    items: Vec<T>,
    jobs: usize,
    fail_fast: bool,
    worker: F,
) -> Vec<OperationResult>
where
    T: Send + 'static,
    F: Fn(T) -> OperationResult + Send + Sync + 'static,
{
    let queue = Arc::new(Mutex::new(VecDeque::from(
        items.into_iter().enumerate().collect::<Vec<_>>(),
    )));
    let results = Arc::new(Mutex::new(Vec::new()));
    let stopped = Arc::new(AtomicBool::new(false));
    let worker = Arc::new(worker);
    let count = jobs.max(1);

    std::thread::scope(|scope| {
        for _ in 0..count {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            let stopped = Arc::clone(&stopped);
            let worker = Arc::clone(&worker);
            scope.spawn(move || {
                loop {
                    if fail_fast && stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let item = queue.lock().expect("work queue lock").pop_front();
                    let Some((index, item)) = item else {
                        break;
                    };
                    let started = std::time::Instant::now();
                    let mut result = worker(item);
                    result.duration_ms = started.elapsed().as_millis();
                    if fail_fast && !result.success {
                        stopped.store(true, Ordering::Relaxed);
                    }
                    results.lock().expect("results lock").push((index, result));
                }
            });
        }
    });

    let mut results = Arc::try_unwrap(results)
        .expect("all result workers joined")
        .into_inner()
        .expect("results lock");
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

pub fn retry_status<F>(retries: u32, mut run: F) -> (u32, io::Result<std::process::ExitStatus>)
where
    F: FnMut() -> io::Result<std::process::ExitStatus>,
{
    let mut attempts = 0;
    loop {
        attempts += 1;
        match run() {
            Ok(status) if status.success() => return (attempts, Ok(status)),
            result if attempts > retries => return (attempts, result),
            _ => std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(attempts))),
        }
    }
}

pub fn local_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn local_sha256_prefix(path: &Path, length: u64) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut remaining = length;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = file
            .read(&mut buffer[..wanted])
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            return Err(format!(
                "{} is shorter than the expected resume prefix ({length} bytes)",
                path.display()
            ));
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn ssh_output(
    host: &str,
    remote: &str,
    options: &TransferOptions,
) -> io::Result<std::process::Output> {
    let mut cmd = Command::new("ssh");
    options.apply_ssh(&mut cmd);
    cmd.arg("--").arg(host).arg(remote).output()
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

pub fn remote_size(
    host: &str,
    path: &str,
    options: &TransferOptions,
) -> Result<Option<u64>, String> {
    let path = remote_shell_path(path);
    let remote = format!(
        "if [ ! -e {path} ]; then exit 3; \
         elif stat -c '%s' {path} >/dev/null 2>&1; then stat -c '%s' {path}; \
         else stat -f '%z' {path}; fi"
    );
    let output = ssh_output(host, &remote, options).map_err(|e| format!("ssh: {e}"))?;
    if output.status.code() == Some(3) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(output_error(&output, "remote stat failed"));
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .map(Some)
        .map_err(|e| format!("invalid remote size: {e}"))
}

pub fn remote_sha256_prefix(
    host: &str,
    path: &str,
    length: u64,
    options: &TransferOptions,
) -> Result<String, String> {
    let path = remote_shell_path(path);
    let remote = format!(
        "if command -v sha256sum >/dev/null 2>&1; then head -c {length} < {path} | sha256sum; \
         elif command -v shasum >/dev/null 2>&1; then head -c {length} < {path} | shasum -a 256; \
         else echo 'snd: no SHA-256 tool found' >&2; exit 127; fi"
    );
    let output = ssh_output(host, &remote, options).map_err(|e| format!("ssh: {e}"))?;
    if !output.status.success() {
        return Err(output_error(&output, "remote prefix checksum failed"));
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "remote prefix checksum returned no output".to_string())
}

pub fn validate_upload_resume(
    host: &str,
    local: &Path,
    remote: &str,
    options: &TransferOptions,
) -> Result<(), String> {
    let Some(remote_length) = remote_size(host, remote, options)? else {
        return Ok(());
    };
    let local_length = local
        .metadata()
        .map_err(|e| format!("{}: {e}", local.display()))?
        .len();
    if remote_length > local_length {
        return Err(format!(
            "remote partial is larger than {} ({} > {} bytes)",
            local.display(),
            remote_length,
            local_length
        ));
    }
    let local_hash = local_sha256_prefix(local, remote_length)?;
    let remote_hash = remote_sha256_prefix(host, remote, remote_length, options)?;
    if local_hash != remote_hash {
        return Err(format!(
            "resume prefix mismatch for {}; remove the remote partial before retrying",
            local.display()
        ));
    }
    Ok(())
}

pub fn validate_download_resume(
    host: &str,
    remote: &str,
    local: &Path,
    options: &TransferOptions,
) -> Result<(), String> {
    let local_length = match local.metadata() {
        Ok(metadata) => metadata.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", local.display())),
    };
    let remote_length = remote_size(host, remote, options)?
        .ok_or_else(|| format!("remote file does not exist: {remote}"))?;
    if local_length > remote_length {
        return Err(format!(
            "local partial {} is larger than the remote file ({} > {} bytes)",
            local.display(),
            local_length,
            remote_length
        ));
    }
    let local_hash = local_sha256_prefix(local, local_length)?;
    let remote_hash = remote_sha256_prefix(host, remote, local_length, options)?;
    if local_hash != remote_hash {
        return Err(format!(
            "resume prefix mismatch for {}; remove the local partial before retrying",
            local.display()
        ));
    }
    Ok(())
}

pub fn acquire_remote_lock(
    host: &str,
    resource: &str,
    options: &TransferOptions,
) -> Result<String, String> {
    let lock = format!("{resource}.snd-lock");
    let remote = format!("mkdir -- {}", remote_shell_path(&lock));
    let output = ssh_output(host, &remote, options).map_err(|e| format!("ssh: {e}"))?;
    if output.status.success() {
        Ok(lock)
    } else {
        Err(format!(
            "destination is locked: {resource} ({})",
            output_error(&output, "lock already exists")
        ))
    }
}

pub fn release_remote_lock(host: &str, lock: &str, options: &TransferOptions) {
    let remote = format!("rmdir -- {}", remote_shell_path(lock));
    let _ = ssh_output(host, &remote, options);
}

pub fn acquire_local_lock(resource: &Path) -> Result<PathBuf, String> {
    let mut name = resource.as_os_str().to_os_string();
    name.push(".snd-lock");
    let lock = PathBuf::from(name);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|e| format!("destination is locked ({}): {e}", lock.display()))?;
    Ok(lock)
}

pub fn release_local_lock(lock: &Path) {
    let _ = std::fs::remove_file(lock);
}

pub fn remote_sha256(host: &str, path: &str, options: &TransferOptions) -> Result<String, String> {
    let quoted = remote_shell_path(path);
    let remote = format!(
        "if command -v sha256sum >/dev/null 2>&1; then sha256sum -- {quoted}; \
         elif command -v shasum >/dev/null 2>&1; then shasum -a 256 -- {quoted}; \
         else echo 'snd: no SHA-256 tool found' >&2; exit 127; fi"
    );
    let mut cmd = Command::new("ssh");
    options.apply_ssh(&mut cmd);
    let output = cmd
        .arg("--")
        .arg(host)
        .arg(remote)
        .output()
        .map_err(|e| format!("ssh: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "remote checksum returned no output".to_string())
}

pub fn remote_rename(
    host: &str,
    from: &str,
    to: &str,
    options: &TransferOptions,
) -> io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("ssh");
    options.apply_ssh(&mut cmd);
    cmd.arg("--")
        .arg(host)
        .arg(format!(
            "mv -f -- {} {}",
            remote_shell_path(from),
            remote_shell_path(to)
        ))
        .status()
}

pub fn sftp_reput(
    host: &str,
    local: &Path,
    remote: &str,
    options: &TransferOptions,
) -> io::Result<std::process::ExitStatus> {
    let flags = if options.preserve { " -p" } else { "" };
    let remote = sftp_remote_path(remote);
    run_sftp_batch(
        host,
        &format!(
            "reput{flags} {} {}\n",
            sftp_quote(&local.to_string_lossy()),
            sftp_quote(&remote)
        ),
        options,
    )
}

pub fn sftp_reget(
    host: &str,
    remote: &str,
    local: &Path,
    options: &TransferOptions,
) -> io::Result<std::process::ExitStatus> {
    let flags = if options.preserve { " -p" } else { "" };
    let remote = sftp_remote_path(remote);
    run_sftp_batch(
        host,
        &format!(
            "reget{flags} {} {}\n",
            sftp_quote(&remote),
            sftp_quote(&local.to_string_lossy())
        ),
        options,
    )
}

fn run_sftp_batch(
    host: &str,
    batch: &str,
    options: &TransferOptions,
) -> io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("sftp");
    options.apply_sftp(&mut cmd);
    if options.json {
        cmd.stdout(Stdio::null());
    }
    let mut child = cmd
        .arg("-b")
        .arg("-")
        .arg("--")
        .arg(host)
        .stdin(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("sftp stdin is piped")
        .write_all(batch.as_bytes())?;
    child.wait()
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn remote_shell_path(path: &str) -> String {
    if path == "~" {
        return "\"$HOME\"".to_string();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return format!("\"$HOME\"/{}", shell_quote(rest));
    }
    if let Some(tilde_path) = path.strip_prefix('~') {
        let (user, rest) = tilde_path
            .split_once('/')
            .map_or((tilde_path, None), |(user, rest)| (user, Some(rest)));
        if !user.is_empty()
            && user
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return match rest {
                Some(rest) => format!("~{user}/{}", shell_quote(rest)),
                None => format!("~{user}"),
            };
        }
    }
    shell_quote(path)
}

fn sftp_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn sftp_remote_path(path: &str) -> String {
    if path == "~" {
        ".".to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        if rest.is_empty() {
            ".".to_string()
        } else {
            rest.to_string()
        }
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_sha256_matches_known_value() {
        let path = std::env::temp_dir().join(format!("snd-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            local_sha256(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn quotes_shell_and_sftp_values() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(sftp_quote("a \\\" b"), "\"a \\\\\\\" b\"");
        assert_eq!(remote_shell_path("~"), "\"$HOME\"");
        assert_eq!(
            remote_shell_path("~/my dir/it's"),
            "\"$HOME\"/'my dir/it'\\''s'"
        );
        assert_eq!(remote_shell_path("~deploy/a b"), "~deploy/'a b'");
        assert_eq!(sftp_remote_path("~/release/file"), "release/file");
        assert_eq!(sftp_remote_path("~"), ".");
    }
}
