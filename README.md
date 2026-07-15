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

### Plan and automate safely

Every upload, download, delete, and sync operation supports `--dry-run`. For
uploads, `snd plan` is a shorter dedicated form that resolves groups, aliases,
relative overrides, and wildcard paths without running `scp`:

```bash
snd plan prod build.jar
snd --dry-run get prod build.jar
snd --dry-run delete prod old.jar
snd --dry-run sync prod ./dist
```

Multi-target operations finish with a success/failure summary. Use `--jobs N`
to fan work out concurrently, `--fail-fast` to stop scheduling after a failure,
and `--retries N` for transient transfer failures.

```bash
snd --jobs 4 --retries 2 prod build.jar
```

Add `--progress` for start/completion timing and native transfer progress. Use
`--audit-log FILE` to append schema-versioned JSON Lines records for completed
operations:

```bash
snd --progress --audit-log .snd-audit.jsonl prod build.jar
snd audit .snd-audit.jsonl
snd audit .snd-audit.jsonl --last 50 --command send --failed
```

`--json` provides machine-readable output for plans, transfer summaries,
`doctor`, `find`, `diff`, and resolved configuration.

### Reliable transfer options

The following typed options are passed safely to the underlying SSH transfer:

- `--preserve` — preserve file modes and timestamps.
- `-C` / `--compress` — enable SSH compression.
- `--limit KBIT/S` — cap transfer bandwidth.
- `-i` / `--identity KEY_FILE` — select an SSH identity.
- `-F` / `--ssh-config FILE` — use an alternate SSH config.
- `--atomic` — upload regular files to temporary names and rename them into
  place only after successful transfer/verification.
- `--verify` — compare SHA-256 after transferring regular files. The remote
  needs `sha256sum` or `shasum`.
- `--resume` — resume partial regular-file transfers using SFTP.

Before resuming, `snd` compares the existing partial file with the matching
prefix of the source. A mismatch is rejected instead of producing a corrupted
result. Atomic and resumable uploads also hold a remote destination lock while
the transfer is active; resumable downloads use a local lock.
Safe resume therefore requires `sha256sum` or `shasum` on the remote, just like
`--verify`.

These compose, so a guarded deployment can be run as:

```bash
snd --atomic --verify --jobs 4 prod build.jar
```

### Undo a direct send

Direct sends automatically snapshot the remote files or directories they are
about to replace. The snapshot also records destinations that did not exist,
so rollback removes newly created files instead of leaving them behind:

```bash
snd dev-proxy ./out/ManaReport.jar
snd rollback dev-proxy             # restore the previous ManaReport.jar
snd rollback dev-proxy ManaReport.jar
```

One snapshot covers the whole send, including sends with multiple inputs. Pass
one or more destination names to restore only those files:

```bash
snd proxy first.jar second.jar third.jar
snd rollback proxy first.jar       # second.jar and third.jar stay deployed
snd rollback proxy                 # restore the remaining two files
```

A named rollback searches backward for the newest unused snapshot containing
that filename, so it also works when the files were sent in separate commands.
Only the restored entries are consumed. A full rollback consumes every
remaining entry in the latest snapshot, so running it again restores the state
before the preceding send. Failed sends attempt an immediate restore and are
not added to the history. The newest 10 successful send snapshots are kept per
remote path by default; use `--backup-keep N` to change that (`0` keeps all),
or `--no-backup` when rollback storage is not wanted. Metadata and payloads
live on the remote host under
`$HOME/.local/share/snd/targets/<destination-hash>/backups`. On a fresh
installation, rollback metadata starts there immediately; it is not first
created inside the destination. Nothing is added to a watched plugin directory
except the files being deployed.

For example, if an added path resolves to `/plugins`, the deployed JAR and its
rollback snapshot are kept separate:

```text
/plugins/ManaReport.jar                                  # deployed file
$HOME/.local/share/snd/targets/<hash>/backups/...        # rollback data
```

There is no `/plugins/.snd` directory.

Inspect every available transaction, or only the history for one destination
name, without downloading the backed-up payloads:

```bash
snd history dev-proxy
snd history dev-proxy ManaReport.jar
snd --json history dev-proxy
```

### Compare and synchronize

`snd diff` compares local files with the names they would have at each remote
target. By default it uses file sizes; `--hash` performs a SHA-256 comparison.

```bash
snd diff prod ./build/app.jar
snd diff --hash prod ./build/app.jar
```

`snd sync` previews and then applies an `rsync` directory synchronization. It
requires `rsync` locally and remotely. Remote deletion is opt-in and always
shown in the preview before confirmation:

```bash
snd sync web ./dist
snd sync --delete web ./dist
snd --dry-run sync --delete prod ./dist
```

Sync supports ordered include/exclude filters and automatically loads
`LOCAL_DIR/.sndignore` as an rsync exclude file when present:

```bash
snd sync --include '*.js' --exclude 'cache/' web ./dist
snd sync --ignore-file ./deploy.ignore web ./dist
snd sync --no-ignore web ./dist
```

Interrupted syncs retain reusable partial files under `.snd-partial`.

### Versioned releases and rollback

`snd release` uploads regular files into an isolated, versioned directory,
verifies every file, writes a completion marker, and then switches
`<target>/.snd/current`. Applications should serve or launch through that
symlink for activation and rollback to take effect.

```bash
snd release --release 2026-07-15 web ./dist/app.jar ./dist/config.json
snd releases web
snd rollback --release web       # activate the recorded previous release
snd rollback --to 2026-07-15 web
```

The previous pointer is updated during both activation and rollback, so a
second `snd rollback --release` toggles back. Without `--release` or `--to`,
rollback first restores the latest direct send and falls back to the previous
versioned release when no direct-send snapshot exists. `--keep N` controls
release retention (default 5), while active and previous releases are always
protected. Release names are never reused unless `--resume` is explicitly
supplied for an incomplete release.

### Deployment manifests

Named operations can be stored in `snd.deploy.toml` and applied together or by
name. File paths are resolved relative to the manifest:

```toml
version = 1

[deploy.web]
target = "prod"
files = ["dist/app.jar", "dist/config.json"]
release = true
atomic = true
verify = true
keep = 5

[deploy.assets]
target = "cdn"
files = ["dist/assets.tar"]
path = "uploads"
```

```bash
snd apply
snd apply ./deploy/production.toml --name web
snd --dry-run --json apply --name web
```

### Project configuration

`snd init` creates a versioned `.snd.toml` in the current directory. The
nearest project configuration is layered over the global config, allowing a
repository to share deployment aliases without copying personal SSH settings.

```bash
snd init
snd config show --resolved              # merged effective TOML
snd --json config show --resolved       # merged effective JSON
snd config --paths      # show global/project config locations
```

Use `--local` with configuration mutations to write the nearest project file
instead of the global configuration:

```bash
snd --local add staging deploy@staging /srv/app
snd --local add-path staging logs /var/log/app
snd --local add-group test staging
```

Configuration utilities validate semantic references after parsing and open
the selected file with `$VISUAL`, `$EDITOR`, or `vi`:

```bash
snd config validate
snd config edit
snd --local config edit
```

Global configuration writes are atomic and retain the previous file as
`servers.toml.bak`.

### JSON and exit codes

Machine output is wrapped in a stable envelope:

```json
{
  "schema_version": 1,
  "command": "send",
  "ok": true,
  "data": []
}
```

Exit code `0` means success, `1` means an operation failed or `diff` found a
difference, and `2` is reserved by the CLI parser for invalid usage. Human
prompts and transfer diagnostics go to stderr when `--json` is active.

### Completion cache

Inspect or clear the asynchronous remote-completion cache without finding its
platform-specific directory manually:

```bash
snd cache show
snd cache clear
snd cache clear --older-than 7
```

### Send files

```bash
snd <server-or-group> [path-alias] <files...>

snd prod plugin.jar
snd staging build.tar.gz config.yml
snd web logs server.log         # uses the "logs" path-alias on web
```

If the first positional matches a path-alias on the chosen server, that path is used instead of the server's default. Otherwise everything is treated as a file.

#### One-off remote directory

Need a path that isn't worth saving as an alias? Put it after the target:

```bash
snd web /tmp/release/ build.tar.gz
snd staging '~/inbox/' notes.md       # quote to keep ~ literal for the remote
snd prod /opt/drop/ build.jar         # group: every member uses /opt/drop
```

The same positional directory works with `ls`, `cat`, `get`/`pull`/`fetch`,
`delete`, and `find`. A trailing `/` makes the directory unambiguous beside file
arguments and is added automatically by completion. `-p` / `--path` remains
available for compatibility and for ambiguous cases such as a bare one-off path.

##### Relative overrides (`./` and `../`)

Prefix the override with `./` or `../` to resolve it relative to the resolved server path instead of replacing it entirely. With a group, each member resolves under its own base.

```bash
# web's default path is /var/www
snd web ./build/ app.jar          # → u@h:/var/www/build
snd web ./logs/today/ error.log   # → u@h:/var/www/logs/today
snd web ../shared/ release.tar    # → u@h:/var/www/../shared (remote resolves)

# Group "prod" with web=/var/www and api=/srv/api
snd prod ./build/ app.jar
# → web sends to /var/www/build, api sends to /srv/api/build
```

`./` and `../` directories resolve relative to each target's configured base.
Anything else is used verbatim. For uploads, an existing local file still wins
over positional-directory detection so multi-file sends keep working.

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

### Diagnose connectivity

`snd doctor` checks cached SSH resolution. `snd doctor --connect` additionally
connects to every server, shows effective SSH host/user/port information,
checks that the default path exists and is writable, verifies required remote
tools, and reports filesystem capacity.

```bash
snd doctor --connect
snd --json doctor --connect
```

### List a remote directory

`snd ls <server> [path]` runs `ls -lhA` over SSH — handy for peeking at a folder
(e.g. the current rotating instance dirs and their IDs) before you send. Without
a path it uses the server's configured default. With no server, `snd ls` still
lists your configured servers and groups.

```bash
snd ls                       # your servers + groups (same as `snd list`)
snd ls app                   # ls the server's default path
snd ls app logs              # ls a named path-alias
snd ls app node              # a glob path lists each matching dir
snd ls app '/srv/app/instances'      # ls a one-off path
```

A group lists every member, labelled per server. Bare paths resolve under each
server's configured path; paths containing `/` and paths beginning with `~` are
used verbatim. `-p` / `--path` remains available for compatibility.

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

Output is colored automatically when stdout is a terminal. `snd ls` colors the
long listing locally, while `find --grep` uses grep's standard color support.
`snd cat` streams through a locally installed `bat` (or `batcat`) for syntax
highlighting; the remote only needs standard `ls`, `grep`, and `cat`. If `bat`
is unavailable, or output is piped/redirected, the content is passed through
unchanged. Set `NO_COLOR=1` to disable color explicitly.

### Get files from a server

```bash
snd get <server-or-group> [path-alias] <files...>
snd pull <server-or-group> [path-alias] <files...>     # alias: pull / fetch

# Pull a file from web's default path into the current dir
snd get web build.tar.gz

# Pull from a named path-alias
snd get web logs error.log

# Pull from a one-off directory
snd get web /var/log/nginx/ error.log

# Absolute / `/`-containing / `~`-prefixed paths are taken as-is
snd get web /etc/nginx/nginx.conf

# Choose a local destination directory
snd get -o ./downloads web build.tar.gz

# Recursive (directories)
snd get -r web stale-build
```

Bare names resolve under the server's path; anything with `/` or `~` is used
verbatim. A positional directory changes the base for the files that follow.
Remote wildcard file operands are rejected because their expanded local
destinations cannot be checked safely for overwrites; request explicit files.
Escape a glob character with a backslash when it is part of the literal remote
filename (for example, `snd get web 'report\[1\].txt'`).

When the target is a group, downloads land in `<dest>/<server-name>/` so files from each member don't collide:

```bash
snd get -o ./dl prod build.tar.gz
# → ./dl/web/build.tar.gz
# → ./dl/api/build.tar.gz
```

If a group references multiple paths on the same server, the path alias is
added to the directory name (for example, `web-default/` and `web-logs/`).

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

The search base is the server's configured path — the same resolution as everything else, so a path-alias or positional directory narrows it:

```bash
snd find prod plugins essentials     # search under the 'plugins' path-alias
snd find prod / worldedit            # sweep the whole server
snd find prod ./logs/ error          # relative to the configured base
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

# Use a one-off directory as the base
snd delete web /tmp/releases/ old-build.jar

# Across an entire group
snd delete prod build.tar.gz config.yml

# Allow directories (recursive)
snd delete -r web stale-build/
```

`snd delete` always stats each target first, lists what it found (size, modified time, full remote path), and prompts before running `rm` on the remote.

For remote names beginning with `-`, place `--` before the filename (for
example, `snd cat web -- -notes.txt` or `snd delete web -- -notes.txt`).

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
- **Remote paths and files** — `snd ls web <TAB>`, `snd cat web <TAB>`, `snd get web <TAB>`, and `snd delete web <TAB>` browse the configured remote path. Directories retain a trailing `/`, and selecting a path-alias changes the directory used by later completions.
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
