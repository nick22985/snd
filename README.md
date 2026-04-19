# snd

Sick of typing out the full `scp user@host:/some/long/path` every time you want to upload a file? `snd` lets you save server presets so you can keep uploading files to specific folders without thinking about it. Define your targets once, then just `snd` your files there.

```
snd prod plugin.jar
# scp plugin.jar -> deploy@10.0.0.1:/opt/app/uploads
```

## Install

```bash
git clone https://github.com/nick22985/snd.git
cd snd
./install.sh
```

Add completions to your shell rc file:

```bash
# Bash (~/.bashrc)
source <(COMPLETE=bash snd)

# Zsh (~/.zshrc)
source <(COMPLETE=zsh snd)

# Fish (~/.config/fish/config.fish)
COMPLETE=fish snd | source
```

## Usage

### Send files

```bash
snd <server> <files...>

snd prod plugin.jar
snd staging build.tar.gz config.yml
```

### Manage servers

```bash
# Add a server (path defaults to ~)
snd add <alias> <host> [/remote/path]
snd add prod deploy@10.0.0.1 /opt/app/uploads
snd add staging devbox

# Edit a server
snd edit <alias> <host> [/remote/path]

# Remove a server
snd remove <alias>
snd rm <alias>

# List configured servers
snd list
snd ls
```

## Shell Completions

Completions are dynamic and context-aware:

- **Server aliases** — `snd <TAB>` completes configured server names
- **SSH hosts** — `snd add myserver <TAB>` fuzzy-matches hosts from `~/.ssh/config` (including `Include`d files), searchable by alias, hostname, IP, or user
- **Remote paths** — `snd add myserver host <TAB>` browses directories on the remote server via SSH
- **Local files** — `snd prod <TAB>` completes local file paths

Remote path completion uses SSH multiplexing (`ControlMaster`) to reuse connections, keeping repeated tab presses fast.

## Config

Server configs are stored in `~/.config/snd/servers.conf`:

```
# snd server config
# Format: alias=host:path
prod=test@10.0.0.1:/home/test/server
staging=devbox:~
```
