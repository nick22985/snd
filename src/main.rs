use clap::{CommandFactory, Parser};
use std::collections::BTreeMap;
use std::io;
use std::process::Command;

use snd::cli::{Cli, Cmd};
use snd::config::{load_servers, save_servers, Server};

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    match cli.command {
        Some(Cmd::Add { alias, host, path }) => {
            let mut servers = load_servers();
            if servers.contains_key(&alias) {
                eprintln!(
                    "Server '{alias}' already exists. Use 'snd edit {alias} <host>' to change host, or 'snd add-path' to add a path."
                );
                std::process::exit(1);
            }
            let path = path.unwrap_or_else(|| "~".to_string()).replace("\\~", "~");
            let mut paths = BTreeMap::new();
            paths.insert("default".to_string(), path.clone());
            servers.insert(
                alias.clone(),
                Server {
                    host: host.clone(),
                    default: "default".to_string(),
                    paths,
                },
            );
            save_servers(&servers).expect("Failed to write config");
            println!("Added: {alias} -> {host}:{path}");
        }
        Some(Cmd::Remove { alias }) => {
            let mut servers = load_servers();
            if servers.remove(&alias).is_none() {
                eprintln!("Server '{alias}' not found.");
                std::process::exit(1);
            }
            save_servers(&servers).expect("Failed to write config");
            println!("Removed: {alias}");
        }
        Some(Cmd::Edit { alias, host }) => {
            let mut servers = load_servers();
            let Some(srv) = servers.get_mut(&alias) else {
                eprintln!("Server '{alias}' not found. Use 'snd add' instead.");
                std::process::exit(1);
            };
            srv.host = host.clone();
            save_servers(&servers).expect("Failed to write config");
            println!("Updated: {alias} host -> {host}");
        }
        Some(Cmd::AddPath {
            server,
            path_alias,
            path,
        }) => {
            let mut servers = load_servers();
            let Some(srv) = servers.get_mut(&server) else {
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
            save_servers(&servers).expect("Failed to write config");
            println!("Added path: {server} {path_alias} -> {path}");
        }
        Some(Cmd::EditPath {
            server,
            path_alias,
            path,
        }) => {
            let mut servers = load_servers();
            let Some(srv) = servers.get_mut(&server) else {
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
            save_servers(&servers).expect("Failed to write config");
            println!("Updated path: {server} {path_alias} -> {path}");
        }
        Some(Cmd::RemovePath { server, path_alias }) => {
            let mut servers = load_servers();
            let Some(srv) = servers.get_mut(&server) else {
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
            save_servers(&servers).expect("Failed to write config");
        }
        Some(Cmd::SetDefault { server, path_alias }) => {
            let mut servers = load_servers();
            let Some(srv) = servers.get_mut(&server) else {
                eprintln!("Server '{server}' not found.");
                std::process::exit(1);
            };
            if !srv.paths.contains_key(&path_alias) {
                eprintln!("Path '{path_alias}' not found on '{server}'.");
                std::process::exit(1);
            }
            srv.default = path_alias.clone();
            save_servers(&servers).expect("Failed to write config");
            println!("Default path for {server}: {path_alias}");
        }
        Some(Cmd::List) => {
            let servers = load_servers();
            if servers.is_empty() {
                println!("No servers configured. Use 'snd add <alias> <host> [path]' to add one.");
                return;
            }
            for (alias, srv) in &servers {
                println!("{alias}  [{}]", srv.host);
                for (name, path) in &srv.paths {
                    let marker = if name == &srv.default { "*" } else { " " };
                    println!("  {marker} {name:<12}  {path}");
                }
            }
        }
        Some(Cmd::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "snd", &mut io::stdout());
        }
        None => {
            let Some(server_name) = cli.server else {
                Cli::command().print_help().ok();
                std::process::exit(1);
            };

            let servers = load_servers();
            let Some(srv) = servers.get(&server_name) else {
                eprintln!("Unknown server: {server_name}");
                eprintln!("Run 'snd list' to see configured servers.");
                std::process::exit(1);
            };

            let mut args = cli.args;
            let target = if let Some(first) = args.first()
                && let Some(path) = srv.path_for(first)
            {
                let path = path.clone();
                args.remove(0);
                format!("{}:{}", srv.host, path)
            } else {
                srv.default_target().unwrap_or_else(|| {
                    eprintln!("Server '{server_name}' has no paths configured.");
                    std::process::exit(1);
                })
            };

            if args.is_empty() {
                eprintln!("No files specified.\nUsage: snd {server_name} [path-alias] <file...>");
                std::process::exit(1);
            }

            if let Some(home) = dirs::home_dir() {
                for arg in &mut args {
                    if let Some(rest) = arg.strip_prefix("~/") {
                        *arg = home.join(rest).to_string_lossy().into_owned();
                    } else if arg == "~" {
                        *arg = home.to_string_lossy().into_owned();
                    }
                }
            }

            println!("scp {} -> {target}", args.join(" "));
            let status = Command::new("scp")
                .args(&args)
                .arg(&target)
                .status()
                .expect("Failed to run scp");

            std::process::exit(status.code().unwrap_or(1));
        }
    }
}
