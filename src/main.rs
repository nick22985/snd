use clap::{CommandFactory, Parser};
use std::collections::BTreeMap;
use std::io;
use std::process::Command;

use snd::cli::{Cli, Cmd};
use snd::config::{
    Config, Group, Server, SshResolved, canonicalize_group_target, load_config, load_config_strict,
    parse_group_target, save_config,
};
use snd::remote::{
    RemoteFileInfo, cat_remote, confirm, destination_basename, expand_remote_glob, find_remote,
    format_size, glob_label, grep_remote, has_glob, join_remote, ls_remote, rm_remote, stat_remote,
};

fn load_or_exit() -> Config {
    load_config_strict().unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    })
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
    let path = match path_alias {
        Some(alias) => srv
            .path_for(alias)
            .ok_or_else(|| format!("Path '{alias}' not found on '{name}'"))?
            .clone(),
        None => srv
            .default_path()
            .ok_or_else(|| format!("Server '{name}' has no paths configured"))?
            .clone(),
    };
    Ok(ResolvedTarget {
        server_name: name.to_string(),
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
    args.iter().filter(|a| !a.starts_with('-')).collect()
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

fn run_scp(target: &ResolvedTarget, args: &[String]) -> i32 {
    let dest = target.target();
    println!("scp {} -> {dest}", args.join(" "));
    let status = Command::new("scp")
        .args(args)
        .arg(&dest)
        .status()
        .expect("Failed to run scp");
    status.code().unwrap_or(1)
}

fn dispatch_send(
    cli_force: bool,
    no_check: bool,
    targets: Vec<ResolvedTarget>,
    args: Vec<String>,
) -> i32 {
    if !no_check && !cli_force {
        let mut any_existing = false;
        for target in &targets {
            match check_existing(target, &args) {
                Ok(existing) => {
                    if !existing.is_empty() {
                        any_existing = true;
                        print_target_block(target, "file(s) already exist", &existing);
                    }
                }
                Err(e) => {
                    eprintln!("[{}] overwrite check failed: {e}", target.server_name);
                    eprintln!("Pass --no-check to skip the remote stat, or -f to force.");
                    return 1;
                }
            }
        }
        if any_existing && !confirm("Overwrite?") {
            eprintln!("Aborted.");
            return 1;
        }
    }

    let mut worst = 0;
    for target in &targets {
        let code = run_scp(target, &args);
        if code != 0 && code > worst {
            worst = code;
        }
    }
    worst
}

fn dispatch_delete(recursive: bool, targets: Vec<ResolvedTarget>, files: Vec<String>) -> i32 {
    if files.is_empty() {
        eprintln!("No files specified for delete.");
        return 1;
    }

    let mut per_target: Vec<(ResolvedTarget, Vec<RemoteFileInfo>, Vec<RemoteFileInfo>)> =
        Vec::new();
    for target in targets {
        let mut remote_paths = Vec::new();
        for f in &files {
            if f.starts_with('-') {
                continue;
            }
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
            println!(
                "[{}] {}:{} — not found: {m}",
                target.server_name, target.host, target.path
            );
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

    for (target, files_only, dirs) in &per_target {
        print_target_block(target, "files to delete", files_only);
        print_target_block(target, "DIRECTORIES to delete (recursive)", dirs);
    }

    let prompt = if any_dirs {
        "This will recursively delete directories. Proceed?"
    } else {
        "Delete these?"
    };
    if !confirm(prompt) {
        eprintln!("Aborted.");
        return 1;
    }

    let mut worst = 0;
    for (target, files_only, dirs) in per_target {
        if !files_only.is_empty() {
            let paths: Vec<String> = files_only.into_iter().map(|i| i.path).collect();
            match rm_remote(&target.host, &paths, false) {
                Ok(status) => {
                    let code = status.code().unwrap_or(1);
                    if code != 0 && code > worst {
                        worst = code;
                    }
                }
                Err(e) => {
                    eprintln!("[{}] ssh rm failed: {e}", target.server_name);
                    worst = 1;
                }
            }
        }
        if !dirs.is_empty() {
            let paths: Vec<String> = dirs.into_iter().map(|i| i.path).collect();
            match rm_remote(&target.host, &paths, true) {
                Ok(status) => {
                    let code = status.code().unwrap_or(1);
                    if code != 0 && code > worst {
                        worst = code;
                    }
                }
                Err(e) => {
                    eprintln!("[{}] ssh rm -r failed: {e}", target.server_name);
                    worst = 1;
                }
            }
        }
    }
    worst
}

const MAX_FIND_LISTED: usize = 200;

fn dispatch_find(
    grep: bool,
    regex: bool,
    case_sensitive: bool,
    depth: Option<u32>,
    targets: Vec<ResolvedTarget>,
    pattern: &str,
) -> i32 {
    let mut worst = 0;
    for target in &targets {
        if grep {
            match grep_remote(&target.host, &target.path, pattern, regex, case_sensitive) {
                Ok(lines) => {
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
                    eprintln!("[{}] search failed: {e}", target.server_name);
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
                            println!(
                                "  {:<40}  {:>10}  {}{}",
                                info.path,
                                format_size(info.size),
                                info.mtime,
                                kind
                            );
                        }
                    }
                    _ => {
                        for p in shown {
                            println!("  {p}");
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
                eprintln!("[{}] search failed: {e}", target.server_name);
                worst = 1;
            }
        }
    }
    worst
}

fn dispatch_ls(targets: Vec<ResolvedTarget>) -> i32 {
    let multi = targets.len() > 1;
    let mut worst = 0;
    for target in &targets {
        if multi {
            println!("[{}] {}:{}", target.server_name, target.host, target.path);
        }
        match ls_remote(&target.host, &target.path) {
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
    let remote_files: Vec<&String> = files.iter().filter(|f| !f.starts_with('-')).collect();
    if remote_files.is_empty() {
        eprintln!("No files specified.");
        return 1;
    }
    let multi = targets.len() > 1;
    let mut worst = 0;
    for target in &targets {
        let paths: Vec<String> = remote_files
            .iter()
            .map(|f| resolve_remote_arg(&target.path, f))
            .collect();
        if multi {
            println!("[{}] {}:{}", target.server_name, target.host, target.path);
        }
        match cat_remote(&target.host, &paths) {
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

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    match cli.command {
        Some(Cmd::Add { alias, host, path }) => {
            let mut cfg = load_config();
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
            save_config(&cfg).expect("Failed to write config");
            println!("Added: {alias} -> {host}:{path}");
        }
        Some(Cmd::Remove { alias }) => {
            let mut cfg = load_config();
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
            save_config(&cfg).expect("Failed to write config");
            println!("Removed: {alias}");
        }
        Some(Cmd::Edit { alias, host }) => {
            let mut cfg = load_config();
            let Some(srv) = cfg.servers.get_mut(&alias) else {
                eprintln!("Server '{alias}' not found. Use 'snd add' instead.");
                std::process::exit(1);
            };
            srv.host = host.clone();
            save_config(&cfg).expect("Failed to write config");
            println!("Updated: {alias} host -> {host}");
        }
        Some(Cmd::AddPath {
            server,
            path_alias,
            path,
        }) => {
            let mut cfg = load_config();
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
            save_config(&cfg).expect("Failed to write config");
            println!("Added path: {server} {path_alias} -> {path}");
        }
        Some(Cmd::EditPath {
            server,
            path_alias,
            path,
        }) => {
            let mut cfg = load_config();
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
            save_config(&cfg).expect("Failed to write config");
            println!("Updated path: {server} {path_alias} -> {path}");
        }
        Some(Cmd::RemovePath { server, path_alias }) => {
            let mut cfg = load_config();
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
            save_config(&cfg).expect("Failed to write config");
        }
        Some(Cmd::SetDefault { server, path_alias }) => {
            let mut cfg = load_config();
            let Some(srv) = cfg.servers.get_mut(&server) else {
                eprintln!("Server '{server}' not found.");
                std::process::exit(1);
            };
            if !srv.paths.contains_key(&path_alias) {
                eprintln!("Path '{path_alias}' not found on '{server}'.");
                std::process::exit(1);
            }
            srv.default = path_alias.clone();
            save_config(&cfg).expect("Failed to write config");
            println!("Default path for {server}: {path_alias}");
        }
        Some(Cmd::List {
            target: Some(name),
            path_alias,
        }) => {
            let cfg = load_config();
            let mut rest: Vec<String> = path_alias.into_iter().collect();
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &name, &mut rest, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    eprintln!("Run 'snd list' to see configured entries.");
                    std::process::exit(1);
                });
            if let Some(extra) = rest.first() {
                eprintln!("Unexpected argument '{extra}' after '{name}'.");
                std::process::exit(1);
            }
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_ls(resolved));
        }
        Some(Cmd::List { target: None, .. }) => {
            let cfg = load_config();
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
            let mut cfg = load_config();
            if cfg.servers.contains_key(&name) {
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
                match canonicalize_group_target(&cfg, t) {
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
            save_config(&cfg).expect("Failed to write config");
            println!("Added group: {name} -> {}", resolved.join(", "));
        }
        Some(Cmd::RemoveGroup { name }) => {
            let mut cfg = load_config();
            if cfg.groups.remove(&name).is_none() {
                eprintln!("Group '{name}' not found.");
                std::process::exit(1);
            }
            save_config(&cfg).expect("Failed to write config");
            println!("Removed group: {name}");
        }
        Some(Cmd::AddToGroup { group, target }) => {
            let mut cfg = load_config();
            let canonical = match canonicalize_group_target(&cfg, &target) {
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
            save_config(&cfg).expect("Failed to write config");
            println!("Added '{canonical}' to group '{group}'");
        }
        Some(Cmd::RemoveFromGroup { group, target }) => {
            let mut cfg = load_config();
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
                save_config(&cfg).expect("Failed to write config");
                println!("Removed '{target}' from group '{group}' (group now empty, deleted).");
            } else {
                save_config(&cfg).expect("Failed to write config");
                println!("Removed '{target}' from group '{group}'");
            }
        }
        Some(Cmd::Get {
            recursive,
            to,
            target,
            files,
        }) => {
            let cfg = load_config();
            let mut files = files;
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
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
            );
            std::process::exit(code);
        }
        Some(Cmd::Delete {
            recursive,
            target,
            files,
        }) => {
            let cfg = load_config();
            let mut resolved = resolve_target_set(&cfg, &target, &mut Vec::new(), false)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            let code = dispatch_delete(recursive, resolved, files);
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
            let cfg = load_config();
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &target, &mut rest, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    eprintln!("Run 'snd list' to see configured entries.");
                    std::process::exit(1);
                });
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            let pattern = match rest.len() {
                0 => {
                    eprintln!(
                        "No search pattern given.\nUsage: snd find [-g] [-e] {target} [path-alias] <pattern>"
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
            let code = dispatch_find(grep, regex, case_sensitive, depth, resolved, &pattern);
            std::process::exit(code);
        }
        Some(Cmd::Cat { target, mut files }) => {
            let cfg = load_config();
            let accept_alias = cli.path.is_none();
            let mut resolved = resolve_target_set(&cfg, &target, &mut files, accept_alias)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    eprintln!("Run 'snd list' to see configured entries.");
                    std::process::exit(1);
                });
            if let Some(p) = cli.path.as_deref() {
                apply_path_override(&mut resolved, p);
            }
            let (resolved, _) = expand_or_exit(resolved);
            std::process::exit(dispatch_cat(resolved, files));
        }
        Some(Cmd::Doctor) => {
            let cfg = load_or_exit();
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
            if !had_issue {
                println!("All servers OK ({} checked).", cfg.servers.len());
            }
            std::process::exit(if had_issue { 1 } else { 0 });
        }
        Some(Cmd::Refresh { alias }) => {
            let mut cfg = load_or_exit();
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
                save_config(&cfg).expect("Failed to write config");
            } else {
                println!("Nothing to update ({} checked).", aliases.len());
            }
        }
        Some(Cmd::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "snd", &mut io::stdout());
        }
        None => {
            let Some(name) = cli.server else {
                Cli::command().print_help().ok();
                std::process::exit(1);
            };

            let cfg = load_config();
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
            }

            if args.is_empty() {
                eprintln!("No files specified.\nUsage: snd {name} [path-alias] <file...>");
                std::process::exit(1);
            }

            let (targets, globbed) = expand_or_exit(targets);
            if globbed
                && !cli.force
                && !confirm(&format!("Send to all {} resolved path(s)?", targets.len()))
            {
                eprintln!("Aborted.");
                std::process::exit(1);
            }

            expand_tildes(&mut args);

            let code = dispatch_send(cli.force, cli.no_check, targets, args);
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

fn dispatch_get(
    cli_force: bool,
    no_check: bool,
    recursive: bool,
    targets: Vec<ResolvedTarget>,
    files: Vec<String>,
    to: Option<&str>,
) -> i32 {
    // Partition out scp flags users may have included (e.g. `-P 22`).
    let (extra_flags, remote_files): (Vec<String>, Vec<String>) =
        files.into_iter().partition(|a| a.starts_with('-'));
    if remote_files.is_empty() {
        eprintln!("No remote files specified.");
        return 1;
    }

    let multi = targets.len() > 1;
    let base = to.unwrap_or(".");

    struct GetPlan {
        target: ResolvedTarget,
        local_dir: std::path::PathBuf,
        items: Vec<(String, std::path::PathBuf)>,
    }

    let mut plans: Vec<GetPlan> = Vec::new();
    for target in targets {
        let local_dir = if multi {
            std::path::PathBuf::from(base).join(&target.server_name)
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
            if !confirm("Overwrite local files?") {
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

    let mut worst = 0;
    for plan in &plans {
        if plan.items.is_empty() {
            continue;
        }
        let sources: Vec<String> = plan
            .items
            .iter()
            .map(|(remote, _)| format!("{}:{}", plan.target.host, remote))
            .collect();
        let dest = plan.local_dir.display().to_string();
        println!("scp {} -> {dest}", sources.join(" "));
        let mut cmd = Command::new("scp");
        if recursive {
            cmd.arg("-r");
        }
        for f in &extra_flags {
            cmd.arg(f);
        }
        for src in &sources {
            cmd.arg(src);
        }
        cmd.arg(&plan.local_dir);
        let status = cmd.status().expect("Failed to run scp");
        let code = status.code().unwrap_or(1);
        if code != 0 && code > worst {
            worst = code;
        }
    }
    worst
}
