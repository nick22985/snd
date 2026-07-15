use std::path::PathBuf;
use std::process::{Command, Output};
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

    fn run_with_home(&self, home: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_snd"))
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("HOME", home)
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
            .env("PATH", path)
            .env_remove("COMPLETE")
            .args(args)
            .output()
            .expect("spawn snd binary")
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
