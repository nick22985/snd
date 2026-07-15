use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestEnv {
    dir: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("snd-it-{}-{n}", std::process::id()));
        std::fs::create_dir_all(dir.join("snd")).unwrap();
        Self { dir }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env_remove("COMPLETE")
            .args(args)
            .output()
            .expect("spawn snd binary")
    }

    fn run_in(&self, cwd: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_snd"))
            .current_dir(cwd)
            .env("XDG_CONFIG_HOME", &self.dir)
            .env_remove("COMPLETE")
            .args(args)
            .output()
            .expect("spawn snd binary")
    }

    fn run_with_home(&self, home: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("HOME", home)
            .env_remove("COMPLETE")
            .args(args)
            .output()
            .expect("spawn snd binary")
    }

    fn run_with_cache(&self, cache: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("XDG_CACHE_HOME", cache)
            .env_remove("COMPLETE")
            .args(args)
            .output()
            .expect("spawn snd binary")
    }

    fn run_with_editor(&self, editor: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("EDITOR", editor)
            .env_remove("VISUAL")
            .env_remove("COMPLETE")
            .args(args)
            .output()
            .expect("spawn snd binary")
    }

    fn run_complete(&self, line: &[&str]) -> String {
        let mut full = vec!["--", "snd"];
        full.extend_from_slice(line);
        let output = Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("COMPLETE", "fish")
            .args(&full)
            .output()
            .expect("spawn snd binary for completion");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn run_complete_with_home(&self, home: &std::path::Path, line: &[&str]) -> String {
        let mut full = vec!["--", "snd"];
        full.extend_from_slice(line);
        let output = Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("HOME", home)
            .env("COMPLETE", "fish")
            .args(&full)
            .output()
            .expect("spawn snd binary for completion");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn run_complete_in(&self, cwd: &std::path::Path, line: &[&str]) -> String {
        let mut full = vec!["--", "snd"];
        full.extend_from_slice(line);
        let output = Command::new(env!("CARGO_BIN_EXE_snd"))
            .current_dir(cwd)
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("COMPLETE", "fish")
            .args(&full)
            .output()
            .expect("spawn snd binary for completion");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn run_complete_with_path(&self, extra_path: &std::path::Path, line: &[&str]) -> String {
        let mut full = vec!["--", "snd"];
        full.extend_from_slice(line);
        let existing = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{existing}", extra_path.display());
        let output = Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("PATH", path)
            .env("COMPLETE", "fish")
            .args(&full)
            .output()
            .expect("spawn snd binary for completion");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn config_file(&self) -> PathBuf {
        self.dir.join("snd").join("servers.toml")
    }

    fn seed_remote_completion(&self, host: &str, directory: &str, entries: &[&str]) {
        let cache = self.dir.join("cache").join("snd");
        std::fs::create_dir_all(&cache).unwrap();
        let key = format!(
            "{}-{}",
            host.replace(['/', '@', ':'], "_"),
            directory.replace('/', "_")
        );
        let mut contents = format!("{directory}\n");
        for entry in entries {
            contents.push_str(entry);
            contents.push('\n');
        }
        std::fs::write(cache.join(&key), contents).unwrap();
        std::fs::write(cache.join(format!("{key}.lock")), "").unwrap();
    }

    fn run_with_path(&self, extra_path: &std::path::Path, args: &[&str]) -> Output {
        let existing = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{existing}", extra_path.display());
        Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("HOME", self.dir.join("home"))
            .env("PATH", path)
            .env_remove("COMPLETE")
            .args(args)
            .output()
            .expect("spawn snd binary")
    }

    fn run_with_path_and_input(
        &self,
        extra_path: &std::path::Path,
        args: &[&str],
        input: &str,
    ) -> Output {
        let existing = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{existing}", extra_path.display());
        let mut child = Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("PATH", path)
            .env_remove("COMPLETE")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn snd binary");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().expect("wait for snd binary")
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn list_empty_says_no_servers() {
    let env = TestEnv::new();
    let out = env.run(&["list"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("No servers configured"));
}

#[test]
fn malformed_config_is_reported_and_not_overwritten() {
    let env = TestEnv::new();
    let malformed = "this is not valid toml = [";
    std::fs::write(env.config_file(), malformed).unwrap();

    let out = env.run(&["add", "web", "u@h", "/var/www"]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("Config error"),
        "stderr: {}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(env.config_file()).unwrap(),
        malformed
    );
}

#[test]
fn project_config_is_layered_over_global_config() {
    let env = TestEnv::new();
    env.run(&["add", "global", "u@g", "/global"]);
    let project = env.dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(".snd.toml"),
        r#"version = 1

[servers.local]
host = "u@l"
default = "default"

[servers.local.paths]
default = "/local"
"#,
    )
    .unwrap();

    let out = env.run_in(&project, &["list"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("global"), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("local"), "stdout: {}", stdout(&out));
}

#[test]
fn init_creates_project_config() {
    let env = TestEnv::new();
    let project = env.dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let out = env.run_in(&project, &["init"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let config = std::fs::read_to_string(project.join(".snd.toml")).unwrap();
    assert!(config.contains("version = 1"), "config: {config}");
}

#[test]
fn local_mutations_write_the_project_config_only() {
    let env = TestEnv::new();
    let project = env.dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    assert!(env.run_in(&project, &["init"]).status.success());

    let out = env.run_in(
        &project,
        &["--local", "add", "staging", "u@h", "/srv/staging"],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let project_config = std::fs::read_to_string(project.join(".snd.toml")).unwrap();
    assert!(project_config.contains("[servers.staging]"));
    assert!(
        !std::fs::read_to_string(env.config_file())
            .unwrap_or_default()
            .contains("[servers.staging]")
    );
}

#[test]
fn config_validate_reports_semantic_errors() {
    let env = TestEnv::new();
    std::fs::write(
        env.config_file(),
        r#"version = 1

[servers.web]
host = "u@h"
default = "missing"

[servers.web.paths]
default = "/srv"
"#,
    )
    .unwrap();
    let out = env.run(&["config", "validate"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("does not exist"),
        "stderr: {}",
        stderr(&out)
    );
}

#[cfg(unix)]
#[test]
fn config_edit_runs_the_editor_and_validates_afterward() {
    use std::os::unix::fs::PermissionsExt;
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/srv"]);
    let editor = env.dir.join("editor");
    std::fs::write(&editor, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out = env.run_with_editor(&editor, &["config", "edit"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("Validated"));
}

#[test]
fn cache_show_and_clear_manage_completion_files() {
    let env = TestEnv::new();
    let cache_root = env.dir.join("cache-root");
    let cache = cache_root.join("snd");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("entry"), "cached").unwrap();

    let show = env.run_with_cache(&cache_root, &["cache", "show"]);
    assert!(show.status.success());
    assert!(stdout(&show).contains("entry"), "stdout: {}", stdout(&show));

    let clear = env.run_with_cache(&cache_root, &["cache", "clear"]);
    assert!(clear.status.success());
    assert!(!cache.join("entry").exists());
}

#[test]
fn manifest_apply_resolves_files_relative_to_the_manifest() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/srv"]);
    let project = env.dir.join("manifest-project");
    std::fs::create_dir_all(project.join("dist")).unwrap();
    std::fs::write(project.join("dist/app.bin"), "payload").unwrap();
    let manifest = project.join("deploy.toml");
    std::fs::write(
        &manifest,
        r#"version = 1

[deploy.web]
target = "web"
files = ["dist/app.bin"]
atomic = true
verify = true
"#,
    )
    .unwrap();

    let out = env.run(&[
        "--dry-run",
        "--json",
        "--no-check",
        "apply",
        manifest.to_str().unwrap(),
        "--name",
        "web",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["command"], "plan");
    assert_eq!(
        value["data"][0]["files"][0],
        project.join("dist/app.bin").to_string_lossy().as_ref()
    );
}

#[test]
fn add_rejects_path_shaped_aliases() {
    let env = TestEnv::new();
    let out = env.run(&["add", "../escape", "u@h", "/srv"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Invalid server alias"));
}

#[test]
fn plan_and_json_dry_run_do_not_invoke_scp() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/srv"]);
    let out = env.run(&["--no-check", "--json", "plan", "web", "missing-local-file"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["command"], "plan");
    assert_eq!(value["data"][0]["action"], "upload");
    assert_eq!(value["data"][0]["target"], "web");
}

#[test]
fn add_creates_server_and_toml() {
    let env = TestEnv::new();
    let out = env.run(&["add", "web", "user@host.example", "/var/www"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let toml_contents = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(
        toml_contents.contains("[servers.web]"),
        "toml: {toml_contents}"
    );
    assert!(toml_contents.contains("host = \"user@host.example\""));
    assert!(toml_contents.contains("default = \"default\""));
    assert!(toml_contents.contains("default = \"/var/www\""));
}

#[test]
fn config_writes_keep_the_previous_file_as_a_backup() {
    let env = TestEnv::new();
    assert!(env.run(&["add", "web", "u@h", "/web"]).status.success());
    assert!(env.run(&["add", "api", "u@h", "/api"]).status.success());

    let backup = std::fs::read_to_string(env.dir.join("snd").join("servers.toml.bak")).unwrap();
    assert!(backup.contains("[servers.web]"), "backup: {backup}");
    assert!(!backup.contains("[servers.api]"), "backup: {backup}");
}

#[test]
fn add_without_path_defaults_to_home() {
    let env = TestEnv::new();
    env.run(&["add", "web", "user@h"]);
    let list = stdout(&env.run(&["list"]));
    assert!(list.contains("~"), "list output: {list}");
}

#[test]
fn add_duplicate_fails() {
    let env = TestEnv::new();
    env.run(&["add", "web", "user@h", "/var/www"]);
    let out = env.run(&["add", "web", "user@h2", "/other"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already exists"));
}

#[test]
fn add_path_and_list() {
    let env = TestEnv::new();
    env.run(&["add", "web", "user@h", "/var/www"]);
    let out = env.run(&["add-path", "web", "logs", "/var/log"]);
    assert!(out.status.success());

    let list = stdout(&env.run(&["list"]));
    assert!(list.contains("default"));
    assert!(list.contains("/var/www"));
    assert!(list.contains("logs"));
    assert!(list.contains("/var/log"));
    assert!(list.contains("* default"));
}

#[test]
fn add_path_duplicate_alias_fails() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-path", "web", "extra", "/b"]);
    let out = env.run(&["add-path", "web", "extra", "/c"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already exists"));
}

#[test]
fn add_path_unknown_server_fails() {
    let env = TestEnv::new();
    let out = env.run(&["add-path", "nope", "p", "/x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn set_default_changes_default() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let out = env.run(&["set-default", "web", "logs"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let list = stdout(&env.run(&["list"]));
    assert!(list.contains("* logs"), "list: {list}");
}

#[test]
fn set_default_unknown_alias_fails() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["set-default", "web", "nope"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn remove_path_resets_default_when_removing_default() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let out = env.run(&["remove-path", "web", "default"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let list = stdout(&env.run(&["list"]));
    assert!(list.contains("* logs"), "list: {list}");
    assert!(!list.contains("default"));
}

#[test]
fn remove_path_refuses_to_remove_last() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["remove-path", "web", "default"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("only path"));
}

#[test]
fn remove_path_unknown_alias_fails() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-path", "web", "extra", "/b"]);
    let out = env.run(&["remove-path", "web", "nope"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn edit_path_changes_target() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["edit-path", "web", "default", "/srv/www"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let list = stdout(&env.run(&["list"]));
    assert!(list.contains("/srv/www"), "list: {list}");
    assert!(!list.contains("/var/www"), "list: {list}");
}

#[test]
fn edit_path_preserves_default_marker() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    env.run(&["edit-path", "web", "default", "/srv/www"]);
    let list = stdout(&env.run(&["list"]));
    assert!(
        list.contains("* default"),
        "default marker preserved: {list}"
    );
    assert!(list.contains("/srv/www"));
}

#[test]
fn edit_path_unknown_server_fails() {
    let env = TestEnv::new();
    let out = env.run(&["edit-path", "nope", "default", "/x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn edit_path_unknown_alias_fails() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["edit-path", "web", "missing", "/x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn edit_path_escapes_tilde_for_shell_passthrough() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["edit-path", "web", "default", "\\~/projects"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(
        toml.contains("\"~/projects\""),
        "expected literal ~/projects, got: {toml}"
    );
}

#[test]
fn edit_changes_host_preserves_paths() {
    let env = TestEnv::new();
    env.run(&["add", "web", "old@host", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let out = env.run(&["edit", "web", "new@host"]);
    assert!(out.status.success());
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(toml.contains("host = \"new@host\""));
    assert!(toml.contains("/var/log"));
}

#[test]
fn edit_refreshes_the_ssh_resolution_cache() {
    let env = TestEnv::new();
    let home = make_home_with_ssh(
        "Host oldalias\n  Hostname old.test\n\nHost newalias\n  Hostname new.test\n  User deploy\n",
    );
    env.run_with_home(&home, &["add", "web", "oldalias", "/var/www"]);

    let edited = env.run_with_home(&home, &["edit", "web", "newalias"]);
    assert!(edited.status.success(), "stderr: {}", stderr(&edited));
    let doctor = env.run_with_home(&home, &["doctor"]);
    assert!(doctor.status.success(), "stderr: {}", stderr(&doctor));

    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(toml.contains("host = \"newalias\""), "toml: {toml}");
    assert!(toml.contains("hostname = \"new.test\""), "toml: {toml}");
    assert!(toml.contains("user = \"deploy\""), "toml: {toml}");
    assert!(!toml.contains("hostname = \"old.test\""), "toml: {toml}");
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn edit_unknown_fails() {
    let env = TestEnv::new();
    let out = env.run(&["edit", "nope", "new@host"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn remove_drops_server() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["remove", "web"]);
    assert!(out.status.success());
    let list = stdout(&env.run(&["list"]));
    assert!(list.contains("No servers configured"));
}

#[test]
fn remove_alias_rm() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["rm", "web"]);
    assert!(out.status.success());
}

#[test]
fn remove_unknown_fails() {
    let env = TestEnv::new();
    let out = env.run(&["remove", "nope"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn dispatch_default_path() {
    let env = TestEnv::new();
    env.run(&["add", "web", "user@h", "/var/www"]);
    let out = env.run(&["web", "missing-local-file-xyz"]);
    assert!(stdout(&out).contains("scp missing-local-file-xyz -> user@h:/var/www"));
}

#[test]
fn dispatch_named_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "deploy", "user@h", "/var/www"]);
    env.run(&["add-path", "deploy", "all", "/plugins/server/all"]);
    let out = env.run(&["deploy", "all", "missing-local-file-xyz"]);
    assert!(
        stdout(&out).contains("scp missing-local-file-xyz -> user@h:/plugins/server/all"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn dispatch_first_arg_not_alias_treated_as_file() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["web", "notanalias.txt"]);
    // Should use default path and pass "notanalias.txt" as file
    assert!(
        stdout(&out).contains("scp notanalias.txt -> u@h:/var/www"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn dispatch_no_files_errors() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["web"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("No files specified"));
}

#[test]
fn dispatch_unknown_server_errors() {
    let env = TestEnv::new();
    let out = env.run(&["nope", "file.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Unknown server"));
}

#[test]
fn completion_server_alias() {
    let env = TestEnv::new();
    env.run(&["add", "deploy", "u@h", "/var/www"]);
    let out = env.run_complete(&[""]);
    assert!(out.contains("deploy"), "completion output: {out}");
}

#[test]
fn completion_path_alias_at_position_0() {
    let env = TestEnv::new();
    env.run(&["add", "deploy", "u@h", "/var/www"]);
    env.run(&["add-path", "deploy", "logs", "/var/log"]);
    env.run(&["add-path", "deploy", "all", "/plugins"]);
    let out = env.run_complete(&["deploy", ""]);
    assert!(out.contains("all"), "completion output: {out}");
    assert!(out.contains("logs"), "completion output: {out}");
    assert!(out.contains("default"), "completion output: {out}");
}

#[test]
fn completion_remote_paths_for_ls_cat_get_and_delete() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/srv/app"]);
    env.run(&["add-path", "web", "logs", "/var/log/app"]);
    env.seed_remote_completion("u@h", "/srv/app", &["config.yml", "plugins/"]);

    for command in ["ls", "cat", "get", "delete"] {
        let out = env.run_complete(&[command, "web", ""]);
        assert!(
            out.contains("/srv/app/config.yml"),
            "{command} should complete remote files: {out}"
        );
        assert!(
            out.contains("/srv/app/plugins/"),
            "{command} should preserve the directory slash: {out}"
        );
        assert!(
            out.lines()
                .any(|line| line == "logs" || line.starts_with("logs\t")),
            "{command} should offer path aliases: {out}"
        );
    }

    let out = env.run_complete(&["find", "web", ""]);
    assert!(
        out.contains("/srv/app/plugins/"),
        "find should complete remote directories: {out}"
    );
    assert!(
        out.lines()
            .any(|line| line == "logs" || line.starts_with("logs\t")),
        "find should offer path aliases: {out}"
    );
}

#[test]
fn completion_remote_files_uses_selected_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/srv/app"]);
    env.run(&["add-path", "web", "logs", "/var/log/app"]);
    env.seed_remote_completion("u@h", "/var/log/app", &["latest.log", "archive/"]);

    let out = env.run_complete(&["cat", "web", "logs", ""]);
    assert!(out.contains("/var/log/app/latest.log"), "completion: {out}");
    assert!(out.contains("/var/log/app/archive/"), "completion: {out}");
    assert!(!out.contains("/srv/app/"), "completion: {out}");

    let out = env.run_complete(&["cat", "web", "/var/log/app", ""]);
    assert!(
        out.contains("/var/log/app/latest.log"),
        "positional directory completion: {out}"
    );
}

#[test]
fn completion_path_alias_for_remove_path() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let out = env.run_complete(&["remove-path", "web", ""]);
    assert!(out.contains("logs"), "completion output: {out}");
    assert!(out.contains("default"), "completion output: {out}");
}

#[test]
fn completion_path_alias_for_set_default() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let out = env.run_complete(&["set-default", "web", ""]);
    assert!(out.contains("logs"));
    assert!(out.contains("default"));
}

#[test]
fn completion_server_for_subcommands() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    for sub in [
        "remove",
        "edit",
        "add-path",
        "edit-path",
        "remove-path",
        "set-default",
    ] {
        let out = env.run_complete(&[sub, ""]);
        assert!(out.contains("web"), "{sub}: {out}");
    }
}

#[test]
fn add_escapes_tilde_for_shell_passthrough() {
    let env = TestEnv::new();
    // Shell usually expands `~` before passing to argv; users double-escape as `\~`.
    let out = env.run(&["add", "web", "u@h", "\\~/foo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(
        toml.contains("\"~/foo\""),
        "expected literal ~/foo in toml, got: {toml}"
    );
}

#[test]
fn add_path_escapes_tilde_for_shell_passthrough() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["add-path", "web", "home", "\\~/projects"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(
        toml.contains("\"~/projects\""),
        "expected literal ~/projects, got: {toml}"
    );
}

#[test]
fn completions_subcommand_emits_script() {
    let env = TestEnv::new();
    for shell in ["bash", "zsh", "fish"] {
        let out = env.run(&["completions", shell]);
        assert!(out.status.success(), "{shell}: {}", stderr(&out));
        let script = stdout(&out);
        assert!(!script.is_empty(), "{shell} script was empty");
        assert!(
            script.contains("snd"),
            "{shell} script missing 'snd' reference"
        );
    }
}

#[test]
fn dispatch_expands_tilde_in_local_args() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);

    let home = std::env::temp_dir().join(format!(
        "snd-arghome-it-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&home).unwrap();
    let home_str = home.to_string_lossy().into_owned();

    // The shell would normally expand `~/foo` before snd sees it; simulate the
    // escaped/quoted case (`\~/foo`, `"~/foo"`) by passing literal `~/...` as argv.
    let out = env.run_with_home(&home, &["web", "~/notes.txt"]);
    let printed = stdout(&out);
    assert!(
        printed.contains(&format!("{home_str}/notes.txt")),
        "expected expanded home path, got: {printed}"
    );
    assert!(
        !printed.contains("~/notes.txt"),
        "tilde should have been expanded: {printed}"
    );

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn completion_tilde_expands_to_home_contents() {
    let env = TestEnv::new();
    env.run(&["add", "deploy", "u@h", "/var/www"]);

    let home = std::env::temp_dir().join(format!(
        "snd-home-it-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("report.md"), "").unwrap();
    std::fs::create_dir(home.join("work")).unwrap();

    // Candidate values must start with the expanded home path, not a literal `~/`.
    // zsh's compadd escapes leading `~` as `\~`, which would leave a stray backslash
    // in the user's command line.
    let home_str = home.to_string_lossy().into_owned();
    let expected_report = format!("{home_str}/report.md");
    let expected_work = format!("{home_str}/work/");

    let out = env.run_complete_with_home(&home, &["deploy", "~/"]);
    assert!(
        out.contains(&expected_report),
        "expected {expected_report} in: {out}"
    );
    assert!(
        out.contains(&expected_work),
        "expected {expected_work} in: {out}"
    );
    assert!(
        !out.contains("~/report.md"),
        "tilde should be expanded to avoid zsh escape: {out}"
    );

    let filtered = env.run_complete_with_home(&home, &["deploy", "~/rep"]);
    assert!(
        filtered.contains(&expected_report),
        "prefix filter should keep report.md: {filtered}"
    );
    assert!(
        !filtered.contains("/work"),
        "prefix filter should drop work/: {filtered}"
    );

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn completion_dot_slash_preserves_prefix() {
    let env = TestEnv::new();
    env.run(&["add", "deploy", "u@h", "/var/www"]);

    let cwd = std::env::temp_dir().join(format!(
        "snd-cwd-it-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("notes.txt"), "").unwrap();
    std::fs::create_dir(cwd.join("out")).unwrap();

    let out = env.run_complete_in(&cwd, &["deploy", "./"]);
    assert!(
        out.contains("./notes.txt"),
        "expected ./notes.txt in: {out}"
    );
    assert!(out.contains("./out/"), "expected ./out/ in: {out}");

    let filtered = env.run_complete_in(&cwd, &["deploy", "./not"]);
    assert!(
        filtered.contains("./notes.txt"),
        "prefix should match: {filtered}"
    );
    assert!(
        !filtered.contains("./out"),
        "prefix should drop out/: {filtered}"
    );

    std::fs::remove_dir_all(&cwd).ok();
}

#[test]
fn add_group_persists_targets_to_toml() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add", "api", "u@h2", "/srv/api"]);
    let out = env.run(&["add-group", "prod", "web", "api"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(toml.contains("[groups.prod]"), "toml: {toml}");
    assert!(toml.contains("\"web\""));
    assert!(toml.contains("\"api\""));
}

#[test]
fn add_group_rejects_unknown_server() {
    let env = TestEnv::new();
    let out = env.run(&["add-group", "prod", "missing"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn add_group_rejects_duplicate_name() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-group", "prod", "web"]);
    let out = env.run(&["add-group", "prod", "web"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already exists"));
}

#[test]
fn add_group_rejects_when_name_collides_with_server() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["add-group", "web", "web"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("server name"));
}

#[test]
fn add_group_validates_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["add-group", "prod", "web:nope"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn add_to_group_appends_target() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add", "api", "u@h2", "/b"]);
    env.run(&["add-group", "prod", "web"]);
    let out = env.run(&["add-to-group", "prod", "api"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(toml.contains("\"api\""));
}

#[test]
fn add_group_resolves_bare_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "spawn", "/srv/spawn/plugins"]);
    let out = env.run(&["add-group", "sb", "spawn"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(toml.contains("\"box1:spawn\""), "toml: {toml}");
}

#[test]
fn add_group_ambiguous_bare_path_alias_errors() {
    let env = TestEnv::new();
    env.run(&["add", "a", "u@h1", "/srv/a"]);
    env.run(&["add-path", "a", "shared", "/srv/a/shared"]);
    env.run(&["add", "b", "u@h2", "/srv/b"]);
    env.run(&["add-path", "b", "shared", "/srv/b/shared"]);
    let out = env.run(&["add-group", "grp", "shared"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("ambiguous"), "stderr: {err}");
    assert!(
        err.contains("'a:shared'") && err.contains("'b:shared'"),
        "stderr: {err}"
    );
}

#[test]
fn add_to_group_resolves_bare_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "spawn", "/srv/spawn/plugins"]);
    env.run(&["add", "web", "u@h2", "/var/www"]);
    env.run(&["add-group", "sb", "web"]);
    let out = env.run(&["add-to-group", "sb", "spawn"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(toml.contains("\"box1:spawn\""), "toml: {toml}");
}

#[test]
fn completion_group_member_offers_path_aliases() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "spawn", "/srv/spawn/plugins"]);
    env.run(&["add-group", "sb", "box1"]);
    let out = env.run_complete(&["add-to-group", "sb", ""]);
    assert!(out.contains("box1"), "completion: {out}");
    assert!(out.contains("spawn"), "completion: {out}");
}

#[test]
fn add_to_group_rejects_duplicate_target() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-group", "prod", "web"]);
    let out = env.run(&["add-to-group", "prod", "web"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already in"));
}

#[test]
fn remove_from_group_drops_target() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add", "api", "u@h2", "/b"]);
    env.run(&["add-group", "prod", "web", "api"]);
    let out = env.run(&["remove-from-group", "prod", "api"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(!toml.contains("\"api\""));
    assert!(toml.contains("\"web\""));
}

#[test]
fn remove_from_group_deletes_empty_group() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-group", "prod", "web"]);
    let out = env.run(&["remove-from-group", "prod", "web"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let toml = std::fs::read_to_string(env.config_file()).unwrap_or_default();
    assert!(!toml.contains("[groups."), "toml: {toml}");
}

#[test]
fn remove_group_succeeds() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-group", "prod", "web"]);
    let out = env.run(&["remove-group", "prod"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

#[test]
fn remove_group_unknown_fails() {
    let env = TestEnv::new();
    let out = env.run(&["remove-group", "ghost"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
}

#[test]
fn remove_server_prunes_group_references() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add", "api", "u@h2", "/b"]);
    env.run(&["add-group", "prod", "web", "api"]);
    env.run(&["remove", "web"]);
    let toml = std::fs::read_to_string(env.config_file()).unwrap_or_default();
    assert!(!toml.contains("\"web\""), "toml: {toml}");
    assert!(toml.contains("\"api\""));
}

#[test]
fn list_shows_groups() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    env.run(&["add-group", "prod", "web"]);
    let out = stdout(&env.run(&["list"]));
    assert!(out.contains("Groups:"), "list: {out}");
    assert!(out.contains("prod"), "list: {out}");
}

#[test]
fn dispatch_group_runs_scp_per_server() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h1", "/var/www"]);
    env.run(&["add", "api", "u@h2", "/srv/api"]);
    env.run(&["add-group", "prod", "web", "api"]);
    let out = env.run(&["--no-check", "prod", "missing-local-file-xyz"]);
    let printed = stdout(&out);
    assert!(
        printed.contains("scp missing-local-file-xyz -> u@h1:/var/www"),
        "stdout: {printed}"
    );
    assert!(
        printed.contains("scp missing-local-file-xyz -> u@h2:/srv/api"),
        "stdout: {printed}"
    );
}

#[test]
fn dispatch_group_with_path_alias_targets() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h1", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log/web"]);
    env.run(&["add", "api", "u@h2", "/srv/api"]);
    env.run(&["add-path", "api", "logs", "/var/log/api"]);
    env.run(&["add-group", "alllogs", "web:logs", "api:logs"]);
    let out = env.run(&["--no-check", "alllogs", "missing-local-file-xyz"]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h1:/var/log/web"), "stdout: {printed}");
    assert!(printed.contains("u@h2:/var/log/api"), "stdout: {printed}");
}

#[test]
fn delete_recursive_flag_is_accepted() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["delete", "-r", "web"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("No files"));
}

#[test]
fn delete_no_files_errors() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["delete", "web"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("No files"));
}

#[test]
fn delete_unknown_target_errors() {
    let env = TestEnv::new();
    let out = env.run(&["delete", "nope", "x.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Unknown server or group"));
}

#[cfg(unix)]
#[test]
fn delete_accepts_a_positional_remote_directory() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let remote = env.dir.join("old-releases");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("old.jar"), "old").unwrap();
    env.run(&["add", "web", "u@h", "/unused"]);

    let directory = format!("{}/", remote.display());
    let out = env.run_with_path(&bin, &["delete", "web", &directory, "old.jar"]);
    assert!(!out.status.success(), "confirmation should default to no");
    assert!(
        stdout(&out).contains(remote.join("old.jar").to_str().unwrap()),
        "stdout: {}",
        stdout(&out)
    );
    assert!(remote.join("old.jar").exists(), "delete should be aborted");
}

#[test]
fn path_override_replaces_default_path() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&[
        "--no-check",
        "-p",
        "/tmp/once",
        "web",
        "missing-local-file-xyz",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("scp missing-local-file-xyz -> u@h:/tmp/once"),
        "stdout: {printed}"
    );
}

#[test]
fn path_override_skips_path_alias_positional() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let out = env.run(&["--no-check", "-p", "/tmp/once", "web", "logs"]);
    let printed = stdout(&out);
    assert!(
        printed.contains("scp logs -> u@h:/tmp/once"),
        "stdout: {printed}"
    );
}

#[test]
fn path_override_applies_to_every_group_member() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h1", "/var/www"]);
    env.run(&["add", "api", "u@h2", "/srv/api"]);
    env.run(&["add-group", "prod", "web", "api"]);
    let out = env.run(&[
        "--no-check",
        "-p",
        "/tmp/release",
        "prod",
        "missing-local-file-xyz",
    ]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h1:/tmp/release"), "stdout: {printed}");
    assert!(printed.contains("u@h2:/tmp/release"), "stdout: {printed}");
}

#[test]
fn path_override_dot_slash_is_relative_to_base() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&[
        "--no-check",
        "-p",
        "./build",
        "web",
        "missing-local-file-xyz",
    ]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h:/var/www/build"), "stdout: {printed}");
}

#[test]
fn path_override_dot_slash_alone_keeps_base() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["--no-check", "-p", "./", "web", "missing-local-file-xyz"]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h:/var/www"), "stdout: {printed}");
    assert!(
        !printed.contains("/var/www/"),
        "shouldn't add trailing slash: {printed}"
    );
}

#[test]
fn path_override_dot_dot_slash_relative_to_base() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&[
        "--no-check",
        "-p",
        "../shared",
        "web",
        "missing-local-file-xyz",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("u@h:/var/www/../shared"),
        "stdout: {printed}"
    );
}

#[test]
fn path_override_dot_slash_resolves_per_group_member() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h1", "/var/www"]);
    env.run(&["add", "api", "u@h2", "/srv/api"]);
    env.run(&["add-group", "prod", "web", "api"]);
    let out = env.run(&[
        "--no-check",
        "-p",
        "./build",
        "prod",
        "missing-local-file-xyz",
    ]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h1:/var/www/build"), "stdout: {printed}");
    assert!(printed.contains("u@h2:/srv/api/build"), "stdout: {printed}");
}

#[test]
fn path_override_unescapes_backslash_tilde() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&[
        "--no-check",
        "-p",
        "\\~/inbox",
        "web",
        "missing-local-file-xyz",
    ]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h:~/inbox"), "stdout: {printed}");
}

#[test]
fn force_flag_is_global() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/a"]);
    let out = env.run(&["-f", "list"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

#[test]
fn completion_fuzzy_narrows_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "deploy", "u@h", "/var/www"]);
    env.run(&["add-path", "deploy", "all", "/plugins"]);
    env.run(&["add-path", "deploy", "logs", "/var/log"]);
    let out = env.run_complete(&["deploy", "al"]);
    assert!(
        out.contains("all"),
        "fuzzy on 'al' should match 'all': {out}"
    );
}

#[test]
fn get_resolves_bare_name_under_server_path() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "web",
        "build.tar.gz",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("u@h:/var/www/build.tar.gz"),
        "stdout: {printed}"
    );
}

#[test]
fn get_passes_through_absolute_remote_path() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "web",
        "/etc/hosts",
    ]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h:/etc/hosts"), "stdout: {printed}");
}

#[test]
fn get_accepts_a_positional_remote_directory() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "web",
        "/var/log/app/",
        "latest.log",
    ]);
    assert!(
        stdout(&out).contains("u@h:/var/log/app/latest.log"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn send_accepts_a_positional_remote_directory() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&[
        "--no-check",
        "web",
        "/srv/releases/",
        "missing-local-file-xyz",
    ]);
    assert!(
        stdout(&out).contains("u@h:/srv/releases/"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        !stdout(&out).contains("scp /srv/releases/"),
        "the remote directory must not be passed to scp: {}",
        stdout(&out)
    );
}

#[test]
fn get_with_recursive_flag_is_accepted() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "get",
        "-r",
        "--to",
        to.to_str().unwrap(),
        "web",
        "stale-build",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("u@h:/var/www/stale-build"),
        "stdout: {printed}"
    );
}

#[test]
fn get_uses_path_alias_when_first_arg_matches() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log/web"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "web",
        "logs",
        "error.log",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("u@h:/var/log/web/error.log"),
        "stdout: {printed}"
    );
}

#[test]
fn get_path_override_skips_alias_positional() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "-p",
        "/tmp/once",
        "get",
        "--to",
        to.to_str().unwrap(),
        "web",
        "logs",
    ]);
    let printed = stdout(&out);
    assert!(printed.contains("u@h:/tmp/once/logs"), "stdout: {printed}");
}

#[test]
fn get_path_override_dot_slash_relative_to_base() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "-p",
        "./build",
        "get",
        "--to",
        to.to_str().unwrap(),
        "web",
        "app.jar",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("u@h:/var/www/build/app.jar"),
        "stdout: {printed}"
    );
}

#[test]
fn get_group_writes_to_per_server_subdir() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h1", "/var/www"]);
    env.run(&["add", "api", "u@h2", "/srv/api"]);
    env.run(&["add-group", "prod", "web", "api"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "prod",
        "build.tar.gz",
    ]);
    let printed = stdout(&out);
    let dest = to.display().to_string();
    assert!(
        printed.contains(&format!("u@h1:/var/www/build.tar.gz -> {dest}/web")),
        "stdout: {printed}"
    );
    assert!(
        printed.contains(&format!("u@h2:/srv/api/build.tar.gz -> {dest}/api")),
        "stdout: {printed}"
    );
    assert!(to.join("web").is_dir(), "expected {}/web", to.display());
    assert!(to.join("api").is_dir(), "expected {}/api", to.display());
}

#[test]
fn get_group_distinguishes_multiple_paths_on_the_same_server() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    env.run(&["add-group", "both", "web", "web:logs"]);
    let to = env.dir.join("dl");

    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "both",
        "same.txt",
    ]);
    let printed = stdout(&out);
    let dest = to.display().to_string();
    assert!(
        printed.contains(&format!("u@h:/var/www/same.txt -> {dest}/web-default")),
        "stdout: {printed}"
    );
    assert!(
        printed.contains(&format!("u@h:/var/log/same.txt -> {dest}/web-logs")),
        "stdout: {printed}"
    );
    assert!(to.join("web-default").is_dir());
    assert!(to.join("web-logs").is_dir());
}

#[test]
fn get_treats_hyphen_prefixed_values_as_remote_files() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let to = env.dir.join("dl");

    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "web",
        "-leading.txt",
    ]);

    assert!(
        stdout(&out).contains("u@h:/var/www/-leading.txt"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn get_rejects_wildcards_that_bypass_overwrite_checks() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["get", "web", "*.log"]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("Wildcard downloads are not supported safely"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn get_accepts_escaped_literal_glob_characters() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["--no-check", "get", "web", r"report\[1\].txt"]);

    assert!(
        stdout(&out).contains("u@h:/var/www/report[1].txt"),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("Wildcard downloads are not supported safely"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn get_no_files_errors() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["--no-check", "get", "web"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("No remote files"));
}

#[test]
fn get_unknown_target_errors() {
    let env = TestEnv::new();
    let out = env.run(&["--no-check", "get", "nope", "x.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Unknown server or group"));
}

#[test]
fn find_unknown_target_errors() {
    let env = TestEnv::new();
    let out = env.run(&["find", "nope", "essentials"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Unknown server or group"));
}

#[test]
fn find_without_pattern_errors() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["find", "web"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("No search pattern"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn find_consumes_path_alias_leaving_no_pattern_errors() {
    // `find web logs` treats "logs" as the path-alias, so no pattern remains.
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log/web"]);
    let out = env.run(&["find", "web", "logs"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("No search pattern"),
        "stderr: {}",
        stderr(&out)
    );
}

#[cfg(unix)]
#[test]
fn find_accepts_a_positional_remote_directory() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let remote = env.dir.join("search-here");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("needle.txt"), "").unwrap();
    env.run(&["add", "web", "u@h", "/unused"]);

    let directory = format!("{}/", remote.display());
    let out = env.run_with_path(&bin, &["find", "web", &directory, "needle"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("needle.txt"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn search_alias_is_accepted() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run(&["search", "web"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("No search pattern"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn completion_find_target_offers_servers() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let out = env.run_complete(&["find", ""]);
    assert!(out.contains("web"), "completion: {out}");
}

#[test]
fn completion_find_offers_path_alias_after_target() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    env.run(&["add-path", "web", "logs", "/var/log"]);
    let out = env.run_complete(&["find", "web", ""]);
    assert!(out.contains("logs"), "completion: {out}");
    assert!(out.contains("default"), "completion: {out}");
}

fn make_home_with_ssh(config: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("snd-ssh-home-{}-{n}", std::process::id()));
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/config"), config).unwrap();
    home
}

#[test]
fn add_populates_resolved_cache_for_known_alias() {
    let env = TestEnv::new();
    let home = make_home_with_ssh("Host myalias\n  Hostname target.test\n  User deploy\n");
    let out = env.run_with_home(&home, &["add", "web", "myalias", "/var/www"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(toml.contains("[servers.web.resolved]"), "toml: {toml}");
    assert!(toml.contains("hostname = \"target.test\""));
    assert!(toml.contains("user = \"deploy\""));
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn add_leaves_cache_empty_for_literal_user_at_host() {
    let env = TestEnv::new();
    let home = make_home_with_ssh("Host other\n  Hostname other.test\n");
    let out = env.run_with_home(&home, &["add", "web", "u@literal", "/var/www"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(!toml.contains("[servers.web.resolved]"), "toml: {toml}");
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn doctor_reports_clean_when_cache_matches() {
    let env = TestEnv::new();
    let home = make_home_with_ssh("Host myalias\n  Hostname target.test\n  User deploy\n");
    env.run_with_home(&home, &["add", "web", "myalias", "/var/www"]);

    let out = env.run_with_home(&home, &["doctor"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("All servers OK"), "{}", stdout(&out));
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn doctor_reports_missing_alias() {
    let env = TestEnv::new();
    let home_before = make_home_with_ssh("Host myalias\n  Hostname target.test\n");
    env.run_with_home(&home_before, &["add", "web", "myalias", "/var/www"]);

    let home_after = make_home_with_ssh("Host other\n  Hostname other.test\n");
    let out = env.run_with_home(&home_after, &["doctor"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("[web]"), "stderr: {err}");
    assert!(err.contains("no longer"), "stderr: {err}");
    std::fs::remove_dir_all(&home_before).ok();
    std::fs::remove_dir_all(&home_after).ok();
}

#[test]
fn doctor_reports_drift() {
    let env = TestEnv::new();
    let home_before = make_home_with_ssh("Host myalias\n  Hostname target.test\n  User deploy\n");
    env.run_with_home(&home_before, &["add", "web", "myalias", "/var/www"]);

    let home_after = make_home_with_ssh("Host myalias\n  Hostname newtarget.test\n  User deploy\n");
    let out = env.run_with_home(&home_after, &["doctor"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("resolves differently"), "stderr: {err}");
    assert!(err.contains("snd refresh web"), "stderr: {err}");
    std::fs::remove_dir_all(&home_before).ok();
    std::fs::remove_dir_all(&home_after).ok();
}

#[test]
fn refresh_updates_cache_after_drift() {
    let env = TestEnv::new();
    let home_before = make_home_with_ssh("Host myalias\n  Hostname target.test\n");
    env.run_with_home(&home_before, &["add", "web", "myalias", "/var/www"]);

    let home_after = make_home_with_ssh("Host myalias\n  Hostname newtarget.test\n");
    let out = env.run_with_home(&home_after, &["refresh", "web"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("[web] updated"));

    let toml = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(
        toml.contains("hostname = \"newtarget.test\""),
        "toml: {toml}"
    );
    assert!(!toml.contains("hostname = \"target.test\""));

    let after = env.run_with_home(&home_after, &["doctor"]);
    assert!(after.status.success(), "stderr: {}", stderr(&after));
    std::fs::remove_dir_all(&home_before).ok();
    std::fs::remove_dir_all(&home_after).ok();
}

#[test]
fn refresh_unknown_server_fails() {
    let env = TestEnv::new();
    let home = make_home_with_ssh("");
    let out = env.run_with_home(&home, &["refresh", "ghost"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"));
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn list_shows_ssh_missing_indicator() {
    let env = TestEnv::new();
    let home_before = make_home_with_ssh("Host myalias\n  Hostname target.test\n");
    env.run_with_home(&home_before, &["add", "web", "myalias", "/var/www"]);

    let home_after = make_home_with_ssh("");
    let list = stdout(&env.run_with_home(&home_after, &["list"]));
    assert!(list.contains("(ssh: missing)"), "list: {list}");
    std::fs::remove_dir_all(&home_before).ok();
    std::fs::remove_dir_all(&home_after).ok();
}

#[test]
fn list_shows_ssh_drift_indicator() {
    let env = TestEnv::new();
    let home_before = make_home_with_ssh("Host myalias\n  Hostname target.test\n");
    env.run_with_home(&home_before, &["add", "web", "myalias", "/var/www"]);

    let home_after = make_home_with_ssh("Host myalias\n  Hostname newtarget.test\n");
    let list = stdout(&env.run_with_home(&home_after, &["list"]));
    assert!(list.contains("(ssh: drift)"), "list: {list}");
    std::fs::remove_dir_all(&home_before).ok();
    std::fs::remove_dir_all(&home_after).ok();
}

#[test]
fn get_pull_alias_works() {
    let env = TestEnv::new();
    env.run(&["add", "web", "u@h", "/var/www"]);
    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "pull",
        "--to",
        to.to_str().unwrap(),
        "web",
        "build.tar.gz",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("u@h:/var/www/build.tar.gz"),
        "stdout: {printed}"
    );
}

#[test]
fn dispatch_unique_path_alias_used_as_top_level() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "creative", "/srv/creative/plugins"]);

    let out = env.run(&["--no-check", "creative", "missing-local-file-xyz"]);
    let printed = stdout(&out);
    assert!(
        printed.contains("scp missing-local-file-xyz -> u@h:/srv/creative/plugins"),
        "stdout: {printed}"
    );
}

#[test]
fn dispatch_ambiguous_path_alias_lists_options() {
    let env = TestEnv::new();
    env.run(&["add", "a", "u@h1", "/srv/a"]);
    env.run(&["add-path", "a", "shared", "/srv/a/shared"]);
    env.run(&["add", "b", "u@h2", "/srv/b"]);
    env.run(&["add-path", "b", "shared", "/srv/b/shared"]);

    let out = env.run(&["--no-check", "shared", "missing-local-file-xyz"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("ambiguous"), "stderr: {err}");
    assert!(err.contains("'a shared'"), "stderr: {err}");
    assert!(err.contains("'b shared'"), "stderr: {err}");
}

#[test]
fn dispatch_server_name_wins_over_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "creative", "u@server", "/var/creative"]);
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "creative", "/srv/creative/plugins"]);

    let out = env.run(&["--no-check", "creative", "missing-local-file-xyz"]);
    let printed = stdout(&out);
    assert!(
        printed.contains("scp missing-local-file-xyz -> u@server:/var/creative"),
        "stdout: {printed}"
    );
}

#[test]
fn dispatch_group_name_wins_over_path_alias() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "creative", "/srv/creative/plugins"]);
    env.run(&["add", "other", "u@h2", "/srv/other"]);
    env.run(&["add-group", "creative", "other"]);

    let out = env.run(&["--no-check", "creative", "missing-local-file-xyz"]);
    let printed = stdout(&out);
    assert!(
        printed.contains("scp missing-local-file-xyz -> u@h2:/srv/other"),
        "stdout: {printed}"
    );
    assert!(
        !printed.contains("/srv/creative/plugins"),
        "stdout: {printed}"
    );
}

#[test]
fn get_unique_path_alias_used_as_top_level() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "creative", "/srv/creative/plugins"]);

    let to = env.dir.join("dl");
    let out = env.run(&[
        "--no-check",
        "get",
        "--to",
        to.to_str().unwrap(),
        "creative",
        "build.tar.gz",
    ]);
    let printed = stdout(&out);
    assert!(
        printed.contains("u@h:/srv/creative/plugins/build.tar.gz"),
        "stdout: {printed}"
    );
}

#[test]
fn delete_unique_path_alias_used_as_top_level() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "creative", "/srv/creative/plugins"]);

    let out = env.run(&["delete", "creative"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("No files"));
}

#[test]
fn completion_unique_path_alias_offered() {
    let env = TestEnv::new();
    env.run(&["add", "box1", "u@h", "/srv"]);
    env.run(&["add-path", "box1", "creative", "/srv/creative/plugins"]);

    let out = env.run_complete(&[""]);
    assert!(out.contains("creative"), "completion: {out}");
}

#[test]
fn completion_ambiguous_path_alias_not_offered() {
    let env = TestEnv::new();
    env.run(&["add", "a", "u@h1", "/srv/a"]);
    env.run(&["add-path", "a", "shared", "/srv/a/shared"]);
    env.run(&["add", "b", "u@h2", "/srv/b"]);
    env.run(&["add-path", "b", "shared", "/srv/b/shared"]);

    let out = env.run_complete(&[""]);
    assert!(out.contains("a"), "completion: {out}");
    assert!(out.contains("b"), "completion: {out}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("shared\t") || *l == "shared"),
        "completion: {out}"
    );
}

/// Write a fake `ssh`/`scp` pair into `bin_dir`. The fake `ssh` ignores all
/// options and the host, then runs the remote command (its last argument)
/// through the local shell — so a glob-expansion command actually expands
/// against local directories the test created. The fake `scp` just succeeds.
#[cfg(unix)]
fn write_fake_ssh_tools(bin_dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(bin_dir).unwrap();
    // `for last; do :; done` leaves `last` holding the final positional arg
    // (the remote command); run it locally so `for p in <glob>` expands here.
    let ssh = "#!/bin/sh\nfor last; do :; done\nsh -c \"$last\"\n";
    let scp = "#!/bin/sh\nexit 0\n";
    for (name, body) in [("ssh", ssh), ("scp", scp)] {
        let p = bin_dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
fn write_fake_copy_ssh_tools(bin_dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(bin_dir).unwrap();
    let ssh = "#!/bin/sh\nfor last; do :; done\nsh -c \"$last\"\n";
    let scp = r#"#!/bin/sh
dest=
for arg do
    case "$arg" in
        -*) ;;
        *:*) dest=${arg#*:} ;;
    esac
done
for src do
    case "$src" in
        -*) continue ;;
        *:*) continue ;;
    esac
    if [ -d "$src" ] && [ -d "$dest" ]; then
        target="$dest/${src##*/}"
        rm -rf -- "$target"
    fi
    cp -R -- "$src" "$dest"
done
"#;
    for (name, body) in [("ssh", ssh), ("scp", scp)] {
        let path = bin_dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
fn write_fake_resume_ssh_tools(bin_dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    write_fake_copy_ssh_tools(bin_dir);
    let sftp = r#"#!/bin/sh
line=$(cat)
eval "set -- $line"
case "$1" in
    reput)
        local=$2
        remote=$3
        offset=0
        if [ -f "$remote" ]; then offset=$(wc -c < "$remote"); fi
        tail -c +$((offset + 1)) "$local" >> "$remote"
        ;;
    reget)
        remote=$2
        local=$3
        offset=0
        if [ -f "$local" ]; then offset=$(wc -c < "$local"); fi
        tail -c +$((offset + 1)) "$remote" >> "$local"
        ;;
    *) exit 2 ;;
esac
"#;
    let path = bin_dir.join("sftp");
    std::fs::write(&path, sftp).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn write_retry_scp(bin_dir: &std::path::Path, counter: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(bin_dir).unwrap();
    let body = format!(
        "#!/bin/sh\necho 'tool progress'\nif [ ! -f '{}' ]; then : > '{}'; exit 1; fi\nexit 0\n",
        counter.display(),
        counter.display()
    );
    let path = bin_dir.join("scp");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn write_fake_rsync(bin_dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(bin_dir).unwrap();
    let path = bin_dir.join("rsync");
    std::fs::write(&path, "#!/bin/sh\necho '>f++++++++ example.txt'\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn write_fake_rsync_with_log(bin_dir: &std::path::Path, log: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(bin_dir).unwrap();
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho '>f++++++++ example.txt'\nexit 0\n",
        log.display()
    );
    let path = bin_dir.join("rsync");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn doctor_connect_checks_remote_path_and_tools() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    env.run(&["add", "web", "u@h", remote.to_str().unwrap()]);

    let out = env.run_with_path(&bin, &["doctor", "--connect"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("connected"),
        "stdout: {}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn diff_reports_same_size_files() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_dir = env.dir.join("local");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    std::fs::write(remote.join("same.txt"), "same").unwrap();
    let local = local_dir.join("same.txt");
    std::fs::write(&local, "same").unwrap();
    env.run(&["add", "web", "u@h", remote.to_str().unwrap()]);

    let out = env.run_with_path(&bin, &["diff", "web", local.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("same"), "stdout: {}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn sync_dry_run_shows_rsync_plan_without_prompting() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_rsync(&bin);
    let local = env.dir.join("dist");
    std::fs::create_dir_all(&local).unwrap();
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(&bin, &["--dry-run", "sync", "web", local.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("sync plan"),
        "stdout: {}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn sync_applies_include_exclude_and_sndignore_filters() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    let log = env.dir.join("rsync-args");
    write_fake_rsync_with_log(&bin, &log);
    let local = env.dir.join("dist");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join(".sndignore"), "*.tmp\n").unwrap();
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(
        &bin,
        &[
            "--dry-run",
            "sync",
            "--include",
            "*.txt",
            "--exclude",
            "cache/",
            "web",
            local.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let args = std::fs::read_to_string(log).unwrap();
    assert!(args.contains("--include\n*.txt"), "args: {args}");
    assert!(args.contains("--exclude\ncache/"), "args: {args}");
    assert!(args.contains("--exclude-from"), "args: {args}");
    assert!(args.contains(".sndignore"), "args: {args}");
}

#[cfg(unix)]
#[test]
fn sync_dry_run_json_is_a_single_machine_readable_document() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_rsync(&bin);
    let local = env.dir.join("dist");
    std::fs::create_dir_all(&local).unwrap();
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(
        &bin,
        &[
            "--dry-run",
            "--json",
            "sync",
            "web",
            local.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["data"][0]["target"], "web");
    assert_eq!(value["data"][0]["changes"][0], ">f++++++++ example.txt");
}

#[cfg(unix)]
#[test]
fn sync_execution_json_suppresses_rsync_progress() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_rsync(&bin);
    let local = env.dir.join("dist");
    std::fs::create_dir_all(&local).unwrap();
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(
        &bin,
        &["--force", "--json", "sync", "web", local.to_str().unwrap()],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["data"][0]["action"], "sync");
    assert_eq!(value["data"][0]["success"], true);
}

#[cfg(unix)]
#[test]
fn atomic_verified_upload_replaces_the_final_file_and_emits_clean_json() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_copy_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_dir = env.dir.join("local");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    let local = local_dir.join("payload.txt");
    std::fs::write(&local, "new payload").unwrap();
    std::fs::write(remote.join("payload.txt"), "old payload").unwrap();
    env.run(&["add", "web", "u@h", remote.to_str().unwrap()]);

    let out = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "--json",
            "--atomic",
            "--verify",
            "web",
            local.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["data"][0]["success"], true);
    assert_eq!(
        std::fs::read_to_string(remote.join("payload.txt")).unwrap(),
        "new payload"
    );
    assert!(std::fs::read_dir(&remote).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".snd-tmp-")
    }));
}

#[cfg(unix)]
#[test]
fn resume_upload_validates_the_existing_prefix_before_appending() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_resume_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_dir = env.dir.join("local");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    let local = local_dir.join("payload.txt");
    std::fs::write(&local, "abcdef").unwrap();
    std::fs::write(remote.join("payload.txt"), "abc").unwrap();
    env.run(&["add", "web", "u@h", remote.to_str().unwrap()]);

    let out = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "--resume",
            "--verify",
            "web",
            local.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(remote.join("payload.txt")).unwrap(),
        "abcdef"
    );
}

#[cfg(unix)]
#[test]
fn resume_upload_rejects_a_mismatched_partial_file() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_resume_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_dir = env.dir.join("local");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    let local = local_dir.join("payload.txt");
    std::fs::write(&local, "abcdef").unwrap();
    std::fs::write(remote.join("payload.txt"), "xyz").unwrap();
    env.run(&["add", "web", "u@h", remote.to_str().unwrap()]);

    let out = env.run_with_path(
        &bin,
        &["--no-check", "--resume", "web", local.to_str().unwrap()],
    );
    assert!(!out.status.success());
    assert!(
        stdout(&out).contains("resume prefix mismatch"),
        "stdout: {} stderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(remote.join("payload.txt")).unwrap(),
        "xyz"
    );
}

#[cfg(unix)]
#[test]
fn release_and_rollback_switch_the_current_symlink() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_copy_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_dir = env.dir.join("local");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    let local = local_dir.join("payload.txt");
    env.run(&["add", "web", "u@h", remote.to_str().unwrap()]);

    std::fs::write(&local, "release one").unwrap();
    let first = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "release",
            "--release",
            "r1",
            "web",
            local.to_str().unwrap(),
        ],
    );
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    std::fs::write(&local, "release two").unwrap();
    let second = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "release",
            "--release",
            "r2",
            "web",
            local.to_str().unwrap(),
        ],
    );
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert_eq!(
        std::fs::read_link(remote.join(".snd/current")).unwrap(),
        std::path::PathBuf::from("releases/r2")
    );
    let listed = env.run_with_path(&bin, &["releases", "web"]);
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));
    assert!(stdout(&listed).contains("active:   r2"));
    assert!(stdout(&listed).contains("previous: r1"));

    let rollback = env.run_with_path(&bin, &["rollback", "web"]);
    assert!(rollback.status.success(), "stderr: {}", stderr(&rollback));
    assert_eq!(
        std::fs::read_link(remote.join(".snd/current")).unwrap(),
        std::path::PathBuf::from("releases/r1")
    );

    let selected = env.run_with_path(&bin, &["rollback", "--to", "r2", "web"]);
    assert!(selected.status.success(), "stderr: {}", stderr(&selected));
    assert_eq!(
        std::fs::read_link(remote.join(".snd/current")).unwrap(),
        std::path::PathBuf::from("releases/r2")
    );
}

#[cfg(unix)]
#[test]
fn direct_send_rollback_restores_the_overwritten_file() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_copy_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_dir = env.dir.join("out");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    let local = local_dir.join("ManaReport.jar");
    let deployed = remote.join("ManaReport.jar");
    std::fs::write(&deployed, "old jar").unwrap();
    std::fs::write(&local, "new jar").unwrap();
    env.run(&["add", "dev-proxy", "u@h", remote.to_str().unwrap()]);

    let send = env.run_with_path(&bin, &["--no-check", "dev-proxy", local.to_str().unwrap()]);
    assert!(send.status.success(), "stderr: {}", stderr(&send));
    assert_eq!(std::fs::read_to_string(&deployed).unwrap(), "new jar");
    assert!(
        !remote.join(".snd").exists(),
        "direct-send metadata must not be stored in the destination"
    );

    std::fs::write(&local, "newest jar").unwrap();
    let second_send =
        env.run_with_path(&bin, &["--no-check", "dev-proxy", local.to_str().unwrap()]);
    assert!(
        second_send.status.success(),
        "stderr: {}",
        stderr(&second_send)
    );
    assert_eq!(std::fs::read_to_string(&deployed).unwrap(), "newest jar");

    let rollback = env.run_with_path(&bin, &["rollback", "dev-proxy"]);
    assert!(
        rollback.status.success(),
        "stdout: {} stderr: {}",
        stdout(&rollback),
        stderr(&rollback)
    );
    assert_eq!(std::fs::read_to_string(&deployed).unwrap(), "new jar");
    assert!(
        stdout(&rollback).contains("restored direct send"),
        "stdout: {}",
        stdout(&rollback)
    );

    let rollback_again = env.run_with_path(&bin, &["rollback", "dev-proxy"]);
    assert!(
        rollback_again.status.success(),
        "stdout: {} stderr: {}",
        stdout(&rollback_again),
        stderr(&rollback_again)
    );
    assert_eq!(std::fs::read_to_string(&deployed).unwrap(), "old jar");
}

#[cfg(unix)]
#[test]
fn direct_send_rollback_restores_a_directory() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_copy_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_parent = env.dir.join("local");
    let local = local_parent.join("plugins");
    let deployed = remote.join("plugins");
    std::fs::create_dir_all(&deployed).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(deployed.join("plugin.jar"), "old plugin").unwrap();
    std::fs::write(local.join("plugin.jar"), "new plugin").unwrap();
    env.run(&["add", "proxy", "u@h", remote.to_str().unwrap()]);

    let send = env.run_with_path(&bin, &["--no-check", "proxy", local.to_str().unwrap()]);
    assert!(send.status.success(), "stderr: {}", stderr(&send));
    assert_eq!(
        std::fs::read_to_string(deployed.join("plugin.jar")).unwrap(),
        "new plugin"
    );

    let rollback = env.run_with_path(&bin, &["rollback", "proxy"]);
    assert!(rollback.status.success(), "stderr: {}", stderr(&rollback));
    assert_eq!(
        std::fs::read_to_string(deployed.join("plugin.jar")).unwrap(),
        "old plugin"
    );
}

#[cfg(unix)]
#[test]
fn direct_send_rollback_removes_a_new_destination() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_copy_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local_dir = env.dir.join("out");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    let local = local_dir.join("new.jar");
    std::fs::write(&local, "new file").unwrap();
    env.run(&["add", "proxy", "u@h", remote.to_str().unwrap()]);

    let send = env.run_with_path(&bin, &["--no-check", "proxy", local.to_str().unwrap()]);
    assert!(send.status.success(), "stderr: {}", stderr(&send));
    assert!(remote.join("new.jar").exists());

    let rollback = env.run_with_path(&bin, &["rollback", "proxy"]);
    assert!(rollback.status.success(), "stderr: {}", stderr(&rollback));
    assert!(!remote.join("new.jar").exists());
}

#[cfg(unix)]
#[test]
fn direct_send_can_rollback_only_one_file_from_a_batch() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_copy_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    let local = env.dir.join("out");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    for name in ["first.jar", "second.jar", "third.jar"] {
        std::fs::write(remote.join(name), format!("old {name}")).unwrap();
        std::fs::write(local.join(name), format!("new {name}")).unwrap();
    }
    env.run(&["add", "proxy", "u@h", remote.to_str().unwrap()]);

    let send = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "proxy",
            local.join("first.jar").to_str().unwrap(),
            local.join("second.jar").to_str().unwrap(),
            local.join("third.jar").to_str().unwrap(),
        ],
    );
    assert!(send.status.success(), "stderr: {}", stderr(&send));

    let history = env.run_with_path(&bin, &["history", "proxy"]);
    assert!(history.status.success(), "stderr: {}", stderr(&history));
    assert!(stdout(&history).contains("first.jar"));
    assert!(stdout(&history).contains("second.jar"));
    assert!(stdout(&history).contains("third.jar"));

    let filtered = env.run_with_path(&bin, &["history", "proxy", "first.jar"]);
    assert!(filtered.status.success(), "stderr: {}", stderr(&filtered));
    assert!(stdout(&filtered).contains("first.jar"));
    assert!(!stdout(&filtered).contains("second.jar"));

    let rollback = env.run_with_path(&bin, &["rollback", "proxy", "first.jar"]);
    assert!(rollback.status.success(), "stderr: {}", stderr(&rollback));
    assert_eq!(
        std::fs::read_to_string(remote.join("first.jar")).unwrap(),
        "old first.jar"
    );
    assert_eq!(
        std::fs::read_to_string(remote.join("second.jar")).unwrap(),
        "new second.jar"
    );
    assert_eq!(
        std::fs::read_to_string(remote.join("third.jar")).unwrap(),
        "new third.jar"
    );

    let remaining = env.run_with_path(&bin, &["rollback", "proxy"]);
    assert!(remaining.status.success(), "stderr: {}", stderr(&remaining));
    assert_eq!(
        std::fs::read_to_string(remote.join("second.jar")).unwrap(),
        "old second.jar"
    );
    assert_eq!(
        std::fs::read_to_string(remote.join("third.jar")).unwrap(),
        "old third.jar"
    );
}

#[cfg(unix)]
#[test]
fn transfer_retries_are_reported_in_json() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    let counter = env.dir.join("attempted");
    write_retry_scp(&bin, &counter);
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "--json",
            "--retries",
            "1",
            "web",
            "missing-local",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["data"][0]["attempts"], 2);
    assert_eq!(value["data"][0]["success"], true);
}

#[cfg(unix)]
#[test]
fn transfer_audit_log_is_json_lines_with_a_schema_version() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    let audit = env.dir.join("audit/operations.jsonl");
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "--audit-log",
            audit.to_str().unwrap(),
            "web",
            "missing-local",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let line = std::fs::read_to_string(&audit).unwrap();
    let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["command"], "send");
    assert_eq!(value["ok"], true);

    let viewed = env.run(&["audit", audit.to_str().unwrap()]);
    assert!(viewed.status.success(), "stderr: {}", stderr(&viewed));
    assert!(stdout(&viewed).contains("send"));
    assert!(stdout(&viewed).contains("OK"));
    assert!(stdout(&viewed).contains("web"));
}

#[cfg(unix)]
#[test]
fn progress_summary_reports_bytes_and_duration() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    let local = env.dir.join("payload.bin");
    std::fs::write(&local, vec![0_u8; 2048]).unwrap();
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(
        &bin,
        &[
            "--no-check",
            "--no-backup",
            "--progress",
            "web",
            local.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("2.00 KB"), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains(" in "), "stdout: {}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn download_json_is_not_prefixed_by_human_progress() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    env.run(&["add", "web", "u@h", "/srv"]);

    let out = env.run_with_path(&bin, &["--no-check", "--json", "get", "web", "payload.txt"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["data"][0]["action"], "download");
    assert_eq!(value["data"][0]["success"], true);
}

#[cfg(unix)]
#[test]
fn parallel_group_send_prints_a_result_summary() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    env.run(&["add", "web", "u@h1", "/srv/web"]);
    env.run(&["add", "api", "u@h2", "/srv/api"]);
    env.run(&["add-group", "prod", "web", "api"]);

    let out = env.run_with_path(
        &bin,
        &["--no-check", "--jobs", "2", "prod", "missing-local"],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("2 succeeded, 0 failed"),
        "stdout: {}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn completion_does_not_execute_host_text_in_the_local_shell() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    let marker = env.dir.join("completion-injected");
    let host = format!("ignored; touch {}; #", marker.display());
    env.run(&["add", "web", &host, remote.to_str().unwrap()]);

    let _ = env.run_complete_with_path(&bin, &["cat", "web", ""]);
    std::thread::sleep(std::time::Duration::from_millis(100));

    assert!(
        !marker.exists(),
        "completion executed configured host text in the local shell"
    );
}

#[cfg(unix)]
#[test]
fn cat_and_delete_support_hyphen_and_pipe_filenames() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    let remote = env.dir.join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    let hyphen = remote.join("-leading.txt");
    let pipe = remote.join("a|b");
    std::fs::write(&hyphen, "hyphen content\n").unwrap();
    std::fs::write(&pipe, "pipe content\n").unwrap();
    env.run(&["add", "web", "u@h", remote.to_str().unwrap()]);

    let cat = env.run_with_path(&bin, &["cat", "web", "--", "-leading.txt"]);
    assert!(cat.status.success(), "stderr: {}", stderr(&cat));
    assert!(stdout(&cat).contains("hyphen content"));

    let delete =
        env.run_with_path_and_input(&bin, &["delete", "web", "--", "-leading.txt", "a|b"], "y\n");
    assert!(delete.status.success(), "stderr: {}", stderr(&delete));
    assert!(!hyphen.exists(), "hyphen-prefixed file was not deleted");
    assert!(!pipe.exists(), "pipe-containing file was not deleted");
    assert!(
        stdout(&delete).contains(&pipe.display().to_string()),
        "delete did not retain the original pipe-containing pathname: {}",
        stdout(&delete)
    );
}

#[cfg(unix)]
#[test]
fn glob_path_fans_out_to_each_matching_dir() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let remote = env.dir.join("instances");
    for id in ["app-1_a1b2c3d4", "app-2_e5f6a7b8", "app-3_9c8d7e6f"] {
        std::fs::create_dir_all(remote.join(id).join("plugins")).unwrap();
    }
    // Non-matching dirs: wrong prefix, and a match with no plugins subdir.
    std::fs::create_dir_all(remote.join("other-1_11223344").join("plugins")).unwrap();
    std::fs::create_dir_all(remote.join("app-9_nodir")).unwrap();

    let pattern = format!("{}/app-*_*/plugins", remote.display());
    env.run(&["add", "app", "user@h", "/unused"]);
    env.run(&["add-path", "app", "node", &pattern]);

    // -f skips the fan-out confirm; the missing local file means no stat ssh.
    let out = env.run_with_path(&bin, &["-f", "app", "node", "missing-local-file-xyz"]);
    let so = stdout(&out);
    let se = stderr(&out);

    // Expansion is reported and lists the matching dirs.
    assert!(se.contains("resolved to 3 path(s)"), "stderr: {se}");

    // One scp per matching plugins dir — and none to the literal pattern.
    for id in ["app-1_a1b2c3d4", "app-2_e5f6a7b8", "app-3_9c8d7e6f"] {
        let expected = format!("user@h:{}/{id}/plugins", remote.display());
        assert!(so.contains(&expected), "missing scp to {id}\nstdout: {so}");
    }
    assert!(
        !so.contains('*'),
        "should not scp to the literal glob: {so}"
    );
    assert!(!so.contains("other-1"), "must not match wrong prefix: {so}");
    assert!(
        !so.contains("app-9"),
        "must skip match without plugins dir: {so}"
    );
}

#[cfg(unix)]
#[test]
fn glob_path_escapes_remote_shell_metacharacters() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    let remote = env.dir.join("targets");
    std::fs::create_dir_all(remote.join("app-one")).unwrap();
    let marker = env.dir.join("glob-injected");
    let pattern = format!("{}/app-*; touch {}; #", remote.display(), marker.display());
    env.run(&["add", "app", "user@h", &pattern]);

    let out = env.run_with_path(&bin, &["-f", "app", "missing-local-file-xyz"]);

    assert!(
        !out.status.success(),
        "escaped composite path should not match"
    );
    assert!(
        !marker.exists(),
        "glob path executed shell metacharacters remotely"
    );
}

#[cfg(unix)]
#[test]
fn glob_path_no_match_is_an_error() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    std::fs::create_dir_all(env.dir.join("services")).unwrap();

    let pattern = format!("{}/services/absent-*/plugins", env.dir.display());
    env.run(&["add", "app", "user@h", "/unused"]);
    env.run(&["add-path", "app", "gone", &pattern]);

    let out = env.run_with_path(&bin, &["-f", "app", "gone", "missing-local-file-xyz"]);
    assert!(!out.status.success(), "should fail when nothing matches");
    assert!(
        stderr(&out).contains("matched no directories"),
        "stderr: {}",
        stderr(&out)
    );
    // Nothing was sent.
    assert!(!stdout(&out).contains("scp "), "stdout: {}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn glob_path_ignores_matching_regular_files() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let remote = env.dir.join("targets");
    std::fs::create_dir_all(remote.join("app-dir")).unwrap();
    std::fs::write(remote.join("app-file"), "not a destination").unwrap();

    let pattern = format!("{}/app-*", remote.display());
    env.run(&["add", "app", "user@h", &pattern]);
    let out = env.run_with_path(&bin, &["-f", "app", "missing-local-file-xyz"]);
    let printed = stdout(&out);

    assert!(printed.contains("app-dir"), "stdout: {printed}");
    assert!(!printed.contains("app-file"), "stdout: {printed}");
    assert!(
        stderr(&out).contains("resolved to 1 path(s)"),
        "stderr: {}",
        stderr(&out)
    );
}

#[cfg(unix)]
#[test]
fn cat_prints_remote_file_contents() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let remote = env.dir.join("app");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("config.yml"), "database.host: 10.0.0.5\n").unwrap();

    env.run(&["add", "proxy", "user@h", remote.to_str().unwrap()]);

    // Bare name resolves under the server path.
    let out = env.run_with_path(&bin, &["cat", "proxy", "config.yml"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("database.host: 10.0.0.5"),
        "stdout: {}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn cat_accepts_a_positional_remote_directory() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let remote = env.dir.join("logs");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("latest.log"), "ready\n").unwrap();
    env.run(&["add", "proxy", "user@h", "/unused"]);

    let directory = format!("{}/", remote.display());
    let out = env.run_with_path(&bin, &["cat", "proxy", &directory, "latest.log"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("ready"), "stdout: {}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn cat_no_files_errors() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);
    env.run(&["add", "proxy", "user@h", "/app"]);
    let out = env.run_with_path(&bin, &["cat", "proxy"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("No files specified"),
        "stderr: {}",
        stderr(&out)
    );
}

#[cfg(unix)]
#[test]
fn ls_lists_remote_directory() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let remote = env.dir.join("instances");
    std::fs::create_dir_all(remote.join("app-1_a1b2c3d4")).unwrap();
    std::fs::create_dir_all(remote.join("app-2_e5f6a7b8")).unwrap();

    env.run(&["add", "app", "user@h", remote.to_str().unwrap()]);
    let out = env.run_with_path(&bin, &["ls", "app"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let so = stdout(&out);
    assert!(so.contains("app-1_a1b2c3d4"), "stdout: {so}");
    assert!(so.contains("app-2_e5f6a7b8"), "stdout: {so}");
}

#[cfg(unix)]
#[test]
fn ls_accepts_path_as_a_positional_argument() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let default = env.dir.join("default");
    let requested = env.dir.join("requested");
    std::fs::create_dir_all(&default).unwrap();
    std::fs::create_dir_all(&requested).unwrap();
    std::fs::create_dir_all(default.join("child")).unwrap();
    std::fs::write(default.join("not-this-one.txt"), "").unwrap();
    std::fs::write(default.join("child").join("relative-path.txt"), "").unwrap();
    std::fs::write(requested.join("positional-path.txt"), "").unwrap();

    env.run(&["add", "app", "user@h", default.to_str().unwrap()]);
    let out = env.run_with_path(&bin, &["ls", "app", requested.to_str().unwrap()]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let so = stdout(&out);
    assert!(so.contains("positional-path.txt"), "stdout: {so}");
    assert!(!so.contains("not-this-one.txt"), "stdout: {so}");
    assert!(!stderr(&out).contains("DEBUG"), "stderr: {}", stderr(&out));

    let out = env.run_with_path(&bin, &["ls", "app", "child"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("relative-path.txt"),
        "stdout: {}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn ls_positional_path_still_accepts_a_path_alias() {
    let env = TestEnv::new();
    let bin = env.dir.join("fakebin");
    write_fake_ssh_tools(&bin);

    let default = env.dir.join("default");
    let logs = env.dir.join("logs");
    std::fs::create_dir_all(&default).unwrap();
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("from-alias.log"), "").unwrap();

    env.run(&["add", "app", "user@h", default.to_str().unwrap()]);
    env.run(&["add-path", "app", "logs", logs.to_str().unwrap()]);
    let out = env.run_with_path(&bin, &["ls", "app", "logs"]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("from-alias.log"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn ls_without_target_still_lists_config() {
    let env = TestEnv::new();
    env.run(&["add", "web", "user@h", "/var/www"]);
    let out = env.run(&["ls"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    // Prints the configured server, not a remote listing.
    assert!(stdout(&out).contains("web"), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("/var/www"),
        "stdout: {}",
        stdout(&out)
    );
}
