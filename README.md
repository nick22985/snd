# snd

Sick of typing out the full `scp user@host:/some/long/path` every time you want to upload a file? `snd` lets you save server presets so you can keep uploading files to specific folders without thinking about it. Define your targets once, then just `snd` your files there.

```
snd prod plugin.jar
# scp plugin.jar -> deploy@10.0.0.1:/opt/app/uploads
```

You can also group servers and send to all of them at once, get a confirmation prompt with file info before overwriting, and delete remote files with the same safety net.

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
snd <server-or-group> [path-alias] <files...>

snd prod plugin.jar
snd staging build.tar.gz config.yml
snd web logs server.log         # uses the "logs" path-alias on web
```

If the first positional matches a path-alias on the chosen server, that path is used instead of the server's default. Otherwise everything is treated as a file.

#### One-off path override

Need a path that isn't worth saving as an alias? Pass `-p` / `--path`:

```bash
snd -p /tmp/release web build.tar.gz
snd --path '~/inbox' staging notes.md      # quote to keep ~ literal for the remote
snd -p /opt/drop prod build.jar            # group: every member uses /opt/drop
```

`-p` overrides whatever the server (or each group member) would otherwise resolve to. When it's set, the first positional is *not* parsed as a path-alias — it's just a file. Works for `snd delete` too.

##### Relative overrides (`./` and `../`)

Prefix the override with `./` or `../` to resolve it relative to the resolved server path instead of replacing it entirely. With a group, each member resolves under its own base.

```bash
# web's default path is /var/www
snd -p ./build web app.jar          # → u@h:/var/www/build
snd -p ./logs/today web error.log   # → u@h:/var/www/logs/today
snd -p ../shared web release.tar    # → u@h:/var/www/../shared (remote resolves)

# Group "prod" with web=/var/www and api=/srv/api
snd -p ./build prod app.jar
# → web sends to /var/www/build, api sends to /srv/api/build
```

`-p ./` alone is a no-op (use the configured base unchanged). Anything that doesn't start with `./` or `../` is taken verbatim, so `-p /abs`, `-p ~/foo`, and `-p plainname` keep working as before.

### Overwrite check

Before scp runs, `snd` SSHs to each target and stats the destination. If a file with the same name already exists you get its size, modified time, and full remote path, then a confirmation prompt:

```
$ snd prod build.tar.gz
[prod] deploy@10.0.0.1:/opt/app/uploads — 1 file(s) already exist:
  /opt/app/uploads/build.tar.gz                12.3 MB  2026-04-30 09:14:02 +0000
Overwrite? [y/N] y
scp build.tar.gz -> deploy@10.0.0.1:/opt/app/uploads
```

Flags:

- `-f` / `--force` — skip the prompt and overwrite without asking.
- `--no-check` — skip the SSH stat entirely (faster on slow links, no prompt).

The stat call reuses your existing SSH multiplexing socket (`~/.ssh/snd-...`), so it doesn't add a fresh connection.

### List a remote directory

`snd ls <server>` runs `ls -lhA` on the server's configured path over SSH — handy
for peeking at a folder (e.g. the current rotating instance dirs and their IDs)
before you send. With no argument, `snd ls` still lists your configured servers
and groups.

```bash
snd ls                       # your servers + groups (same as `snd list`)
snd ls app                   # ls the server's default path
snd ls app logs              # ls a named path-alias
snd ls app node              # a glob path lists each matching dir
snd ls -p '/srv/app/instances' app   # one-off path
```

A group lists every member, labelled per server. Glob paths and `-p` resolve the
same way as everywhere else.

### Print remote file contents

`snd cat <server> <files...>` prints remote files to stdout (`cat` over SSH) —
no download, pipe- and redirect-friendly:

```bash
snd cat prod config.yml                 # bare name → under the server path
snd cat prod /etc/nginx/nginx.conf      # /-or-~ path → verbatim
snd cat prod logs latest.log            # from a path-alias
snd cat -p /var/log prod app.log        # one-off path override
snd cat prod config.yml | grep host     # it's just stdout
```

Paths resolve exactly like `get`/`delete` (bare names under the server path,
anything with `/` or `~` verbatim, `-p` overrides, globs expand). For a group,
each server's output is printed under a `[server] host:path` header; a single
server is a clean passthrough with no header.

### Get files from a server

```bash
snd get <server-or-group> [path-alias] <files...>
snd pull <server-or-group> [path-alias] <files...>     # alias: pull / fetch

# Pull a file from web's default path into the current dir
snd get web build.tar.gz

# Pull from a named path-alias
snd get web logs error.log

# Absolute / `/`-containing / `~`-prefixed paths are taken as-is
snd get web /etc/nginx/nginx.conf

# Choose a local destination directory
snd get -o ./downloads web build.tar.gz

# Recursive (directories)
snd get -r web stale-build
```

Bare names resolve under the server's path; anything with `/` or `~` is used verbatim. `-p` / `--path` works the same as for `snd` and `snd delete`, including the `./` and `../` relative forms.

When the target is a group, downloads land in `<dest>/<server-name>/` so files from each member don't collide:

```bash
snd get -o ./dl prod build.tar.gz
# → ./dl/web/build.tar.gz
# → ./dl/api/build.tar.gz
```

Before scp runs, `snd get` checks each local destination and lists any existing files (size, age, full path) so you can confirm before they're overwritten:

```
$ snd get web build.tar.gz
Local file(s) already exist:
  ./build.tar.gz                              12.3 MB  4h ago
Overwrite local files? [y/N]
```

`-f` skips the prompt, `--no-check` skips the local check entirely.

### Search a server

Not sure which folder a jar landed in? `snd find` runs a search over SSH and prints where things are, so you can copy a path straight into `snd -p`.

```bash
snd find <server-or-group> [path-alias] <pattern>
snd search ...                                    # alias
```

By default it matches **file names**, case-insensitively, as a substring — so `essentials` finds `EssentialsX.jar` anywhere under the base:

```
$ snd find prod essentials
[prod] deploy@10.0.0.1:/opt/app — 2 match(es):
  /opt/app/plugins/EssentialsX.jar             8.2 MB  2026-07-01 09:14:02 +0000
  /opt/app/backup/EssentialsX-old.jar          7.9 MB  2026-05-20 22:03:11 +0000
```

The search base is the server's configured path — the same resolution as everything else, so a path-alias narrows it and `-p` overrides it:

```bash
snd find prod plugins essentials     # search under the 'plugins' path-alias
snd find -p / prod worldedit         # sweep the whole server
snd find -p ./logs prod error        # relative to the configured base
```

Flags:

- **`-e` / `--regex`** — treat the pattern as an extended regex instead of a substring. It matches anywhere in the path unless you anchor it with `^` / `$`.
- **`--case-sensitive`** — turn off the default case-insensitivity.
- **`-d` / `--depth N`** — limit a filename search to `N` directory levels below the base (handy with `-p /`).
- **`-g` / `--grep`** — search **file contents** instead of names (recursive `grep`, skips binaries). Output is `path:line:content`:

```
$ snd find -g prod "database.host"
[prod] deploy@10.0.0.1:/opt/app
  /opt/app/config.yml:12:  database.host: 10.0.0.5
```

`-e` and `--case-sensitive` apply to `-g` too (extended regex / case-sensitive grep). A plain pattern (no `-e`) is matched as a fixed string, so `.` and other regex characters are literal.

If the first positional after the server matches a path-alias it's consumed as the base; quote or use `-p` if you actually want to search for that word. Searching a **group** runs the search on every member and labels the results per server.

### Delete remote files

```bash
# Delete files (paths resolved under the server's configured path)
snd delete web build.tar.gz

# Bare names → resolved under the server path. Anything with `/` or `~` → used as-is.
snd delete web /tmp/dump.sql

# Across an entire group
snd delete prod build.tar.gz config.yml

# Allow directories (recursive)
snd delete -r web stale-build/
```

`snd delete` always stats each target first, lists what it found (size, modified time, full remote path), and prompts before running `rm` on the remote.

Safeguards:

- **Confirmation is unconditional.** `-f` does *not* skip the delete prompt.
- **Directories are refused by default.** Anything that stats as a directory is listed and skipped unless you pass `-r` / `--recursive`.
- **Plain `rm` for files, `rm -rf` only for explicit dirs.** The two run as separate commands on the remote, so a misclassified file can never be recursively wiped.
- **Louder prompt for recursive deletes.** When directories are in the mix the prompt makes it obvious before you hit `y`.

Example:

```
$ snd delete -r web build.tar.gz old-cache/
[web] deploy@10.0.0.1:/opt/app/uploads — files to delete (1):
  /opt/app/uploads/build.tar.gz                12.3 MB  2026-04-30 09:14:02
[web] deploy@10.0.0.1:/opt/app/uploads — DIRECTORIES to delete (recursive) (1):
  /opt/app/uploads/old-cache                    4.0 KB  2026-04-29 22:00:11 (dir)
This will recursively delete directories. Proceed? [y/N]
```

### Groups

A group is a named set of servers. Sending to a group sends to each member sequentially. Each entry is `server` (uses that server's default path) or `server:path_alias` (pins to a specific path on that server).

```bash
# Create a group from one or more targets
snd add-group prod web api db
snd add-group alllogs web:logs api:logs

# Manage members
snd add-to-group prod cache
snd remove-from-group prod cache
snd remove-group prod

# Send to every server in the group
snd prod build.tar.gz

# Delete on every server in the group
snd delete prod stale.log
```

Removing a server (`snd remove web`) automatically prunes that server from any group that referenced it; if a group becomes empty it's deleted too.

The overwrite check runs per-member: each existing destination is listed under its server, and a single `Overwrite?` prompt covers the whole batch.

### Manage servers

```bash
# Add a server (path defaults to ~)
snd add <alias> <host> [/remote/path]
snd add prod deploy@10.0.0.1 /opt/app/uploads
snd add staging devbox

# Change the host of an existing server (paths preserved)
snd edit <alias> <host>

# Remove a server (also drops it from any groups)
snd remove <alias>     # alias: snd rm

# List configured servers and groups
snd list               # alias: snd ls
# (with a target, `snd ls <server>` lists that server's remote dir — see above)
```

### Manage paths

Each server has a map of named paths and a default. Use path aliases to keep a single server pointed at multiple destinations.

```bash
# Add another path on an existing server
snd add-path web logs /var/log/nginx

# Edit / remove paths
snd edit-path   web logs /var/log/web
snd remove-path web logs        # alias: snd rm-path

# Pick which path is the default for `snd web <files>`
snd set-default web logs
```

In `snd list`, the active default is marked with `*`:

```
web  [deploy@10.0.0.1]
  * default       /opt/app/uploads
    logs          /var/log/nginx

Groups:
prod
    web
    api:logs
```

### Wildcard paths

A remote path can contain a shell glob (`*`, `?`, `[...]`). Before running,
`snd` SSHes to the server and expands the pattern to the directories that
actually exist, then fans the operation out to every match. This is built for
setups where the live directory carries an ID suffix that changes on every
restart:

```
/srv/app/instances/          # live dirs, ID changes each start
  app-1_a1b2c3d4/  app-2_e5f6a7b8/  app-3_9c8d7e6f/  app-4_0f1e2d3c/  ...
```

Point one path at the whole set with a glob and let `snd` work out the current
IDs:

```bash
snd add-path app node '/srv/app/instances/app-*_*/plugins'

snd app node build.jar
# [app] /srv/app/instances/app-*_*/plugins — resolved to 4 path(s) on deploy@10.0.0.1:
#     /srv/app/instances/app-1_a1b2c3d4/plugins
#     /srv/app/instances/app-2_e5f6a7b8/plugins
#     /srv/app/instances/app-3_9c8d7e6f/plugins
#     /srv/app/instances/app-4_0f1e2d3c/plugins
# Send to all 4 resolved path(s)? [y/N] y
```

Notes:

- Quote the pattern when adding it (`'...*...'`) so your local shell doesn't
  expand it — you want the `*` stored in the config and expanded on the remote.
- A single glob path already fans out to every match, so you usually don't need
  a group for "all the instance folders". Groups still compose with globs if
  you want to mix in other servers.
- Only directories that exist are matched. A pattern that matches nothing is an
  error (nothing is sent), so a typo can't silently no-op.
- The fan-out is confirmed before a `send` (skip with `-f`). `get`, `delete`,
  and `find` expand the same way; `get` drops each match's files into a
  per-match subdirectory so they don't collide.
- Works via `-p` too: `snd -p '/srv/app/instances/app-*_*/plugins' app build.jar`.

## Shell Completions

Completions are dynamic and context-aware:

- **Servers and groups** — `snd <TAB>` completes both, with a hint showing each entry's host or member count.
- **Path aliases** — `snd web <TAB>` completes path-aliases configured on `web`.
- **SSH hosts** — `snd add myserver <TAB>` fuzzy-matches hosts from `~/.ssh/config` (including `Include`d files), searchable by alias, hostname, IP, or user.
- **Remote paths** — `snd add myserver host <TAB>` browses directories on the remote server via SSH.
- **Group members** — `snd remove-from-group prod <TAB>` lists current members.
- **Local files** — `snd prod <TAB>` completes local file paths.

Remote path completion uses SSH multiplexing (`ControlMaster`) to reuse connections, keeping repeated tab presses fast.

## Config

Configs are stored as TOML in `~/.config/snd/servers.toml`:

```toml
[servers.web]
host = "deploy@10.0.0.1"
default = "default"

[servers.web.paths]
default = "/opt/app/uploads"
logs = "/var/log/nginx"

[servers.api]
host = "deploy@10.0.0.2"
default = "default"

[servers.api.paths]
default = "/srv/api"
logs = "/var/log/api"

[groups.prod]
targets = ["web", "api"]

[groups.alllogs]
targets = ["web:logs", "api:logs"]
```

The CLI is the source of truth — you don't have to edit this file by hand, but it's plain TOML if you want to.
