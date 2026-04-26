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
            .env("COMPLETE", "fish")
            .args(&full)
            .output()
            .expect("spawn snd binary for completion");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn config_file(&self) -> PathBuf {
        self.dir.join("snd").join("servers.toml")
    }

    fn legacy_file(&self) -> PathBuf {
        self.dir.join("snd").join("servers.conf")
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
    assert!(toml_contents.contains("[web]"));
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
    assert!(list.contains("* default"), "default marker preserved: {list}");
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
fn legacy_conf_is_migrated() {
    let env = TestEnv::new();
    std::fs::write(
        env.legacy_file(),
        "# snd server config\nweb=user@h:/var/www\ndb=dbhost:/opt/db\n",
    )
    .unwrap();
    let out = env.run(&["list"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let list = stdout(&out);
    assert!(list.contains("web"));
    assert!(list.contains("user@h"));
    assert!(list.contains("db"));
    assert!(list.contains("dbhost"));
    assert!(env.config_file().exists(), "TOML file should be created");
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
    for sub in ["remove", "edit", "add-path", "edit-path", "remove-path", "set-default"] {
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
