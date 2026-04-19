use clap::{CommandFactory, Parser};
use std::io;
use std::process::Command;

use snd::cli::{Cli, Cmd};
use snd::config::{load_servers, save_servers};

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    match cli.command {
        Some(Cmd::Add { alias, host, path }) => {
            let mut servers = load_servers();
            if servers.contains_key(&alias) {
                eprintln!("Server '{alias}' already exists. Use 'snd edit {alias} <host> [path]' to modify.");
                std::process::exit(1);
            }
            let path = path.replace("\\~", "~");
            let target = format!("{host}:{path}");
            servers.insert(alias.clone(), target.clone());
            save_servers(&servers).expect("Failed to write config");
            println!("Added: {alias} -> {target}");
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
        Some(Cmd::Edit { alias, host, path }) => {
            let mut servers = load_servers();
            if !servers.contains_key(&alias) {
                eprintln!("Server '{alias}' not found. Use 'snd add' instead.");
                std::process::exit(1);
            }
            let path = path.replace("\\~", "~");
            let target = format!("{host}:{path}");
            servers.insert(alias.clone(), target.clone());
            save_servers(&servers).expect("Failed to write config");
            println!("Updated: {alias} -> {target}");
        }
        Some(Cmd::List) => {
            let servers = load_servers();
            if servers.is_empty() {
                println!("No servers configured. Use 'snd add <alias> <target>' to add one.");
                return;
            }
            let max_len = servers.keys().map(|k| k.len()).max().unwrap_or(10).max(5);
            println!("{:<max_len$}  {}", "ALIAS", "TARGET");
            println!("{:<max_len$}  {}", "-----", "------");
            for (alias, target) in &servers {
                println!("{alias:<max_len$}  {target}");
            }
        }
        Some(Cmd::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "snd", &mut io::stdout());
        }
        None => {
            let Some(server) = cli.server else {
                Cli::command().print_help().ok();
                std::process::exit(1);
            };

            let files = cli.files;
            if files.is_empty() {
                eprintln!("No files specified.\nUsage: snd {server} <file...>");
                std::process::exit(1);
            }

            let servers = load_servers();
            let Some(target) = servers.get(&server) else {
                eprintln!("Unknown server: {server}");
                eprintln!("Run 'snd list' to see configured servers.");
                std::process::exit(1);
            };

            println!("scp {} -> {target}", files.join(" "));
            let status = Command::new("scp")
                .args(&files)
                .arg(target)
                .status()
                .expect("Failed to run scp");

            std::process::exit(status.code().unwrap_or(1));
        }
    }
}
