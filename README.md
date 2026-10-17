# termnest

Personal SSH client for Windows 10/11 built with Tauri 2, Rust and Svelte.
Key-based authentication only, low memory footprint, local storage.

Full specification: [SPEC.md](SPEC.md).

## Features

- Terminal on xterm.js (WebGL renderer with DOM fallback, truecolor, search, clickable
  links, copy on select, multi-line paste warning).
- Server manager: groups with subgroups, drag and drop to reorder servers and groups
  or move them between groups, search, quick connect `user@host:port`, import from
  `~/.ssh/config`.
- Password logins per server: the edit dialog asks the server itself which login
  methods it offers and can forbid or allow logins without a key (sshd drop-in,
  checked with `sshd -t`, rolled back if key login stops working).
- Host key verification against termnest's own `known_hosts`: unknown hosts ask for
  the SHA256 fingerprint, changed keys are blocked until the old entry is removed.
- Keys: list of `~/.ssh` keys, generation (Ed25519 / RSA 4096, owner-only ACL),
  passphrase prompts kept in memory only, wizard that installs a key on a server
  with a one-time password and optionally disables password logins.
- Tabs and splits (arbitrary nesting), several windows, moving tabs between windows
  without reconnecting.
- Layout saved to `layout.json` and restored on start; automatic reconnect with
  1, 2, 4 ... 30 s backoff.
- Snippets with `{{variables}}`, palette (Ctrl+Shift+P), sending to the current,
  selected or all panels, broadcast mode.
- SFTP side panel over the same SSH connection: navigation, upload (also drag and drop
  from Explorer), download, rename, delete, chmod, transfer queue.
- Themes, fonts, remappable hotkeys.

Passwords and passphrases are never written to disk.

## Claude Code integration

Requires the Claude Code CLI installed and logged in (`claude` in PATH or a path in
Settings > Claude integration).

- **Projects.** A server, a group or a single panel can be linked to a local project
  folder (panel > server > group). The project is shown in the panel header.
- **MCP server** (off by default). Settings > Claude integration shows the
  `claude mcp add ...` command. Read-only tools: `list_hosts`, `list_sessions`,
  `read_output`, `read_selection`, `sftp_list`, `sftp_read`, `server_info`. Actions
  `exec` and `send_input` run only after approval in termnest. Listens on 127.0.0.1
  only, bearer token in `%APPDATA%\termnest\mcp-token`.
- **Send to Claude** (Ctrl+Shift+A or the button in the panel header): the selection or
  the last lines of output go to a Claude Code agent started in the project folder; for
  git projects each task gets its own worktree and branch `termnest/fix-<id>`. Progress,
  changed files, follow-ups, diff and stop are in the Agents section.

## Data

`%APPDATA%\termnest\`: `hosts.toml`, `snippets.toml`, `settings.toml`, `layout.json`,
`known_hosts`. Files are written atomically; a broken file is moved aside as
`*.broken-<time>` and the app starts with defaults.

## Default hotkeys

| Action | Keys |
|---|---|
| New tab (server picker) | Ctrl+Shift+T |
| Close panel/tab | Ctrl+Shift+W |
| New window | Ctrl+Shift+N |
| Split right / down | Alt+Shift+= / Alt+Shift+- |
| Focus between panels | Alt+arrows |
| Next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Duplicate session | Ctrl+Shift+D |
| Snippet palette | Ctrl+Shift+P |
| Search in terminal | Ctrl+Shift+F |
| Copy / paste | Ctrl+Shift+C / Ctrl+Shift+V, right click |
| Broadcast mode | Ctrl+Shift+B |
| SFTP panel | Ctrl+Shift+S |
| Sidebar | Ctrl+Shift+E |

All of them can be changed in Settings.

## Project layout

```
src/                    Svelte + TypeScript frontend
src/lib/terminal/       xterm.js wrapper
src/lib/layout/         split geometry
src/components/         UI components
src/stores/             window state
src-tauri/              Tauri app crate: commands, windows, prompts
src-tauri/core/         termnest-core: sessions, hosts, keys, layout, SFTP (no Tauri)
docker/                 Build/test container (cross-compilation for Windows)
scripts/build.ps1       One-command Docker build or test run
```

## Building with Docker (main path)

Requires only Docker Desktop (Linux containers). The container cross-compiles
for `x86_64-pc-windows-msvc` with cargo-xwin; the app is never run inside it.

```powershell
./scripts/build.ps1          # release build
./scripts/build.ps1 -Test    # svelte-check, eslint, rustfmt, core tests, clippy
```

Results are copied to `release/`:

- `termnest.exe` - portable executable;
- `termnest_<version>_x64-setup.exe` - NSIS installer.

Only the NSIS installer can be produced from Linux; `.msi` (WiX) requires the
native Windows build below.

The first run builds the image and downloads dependencies, later runs reuse
the named volumes (`node_modules`, cargo registry, `target`, xwin cache).
cargo-xwin downloads the Microsoft CRT and Windows SDK on first use; by building
this way you accept the Microsoft license terms for those files.

To drop all caches: `docker compose down --volumes`.

## Building natively on Windows

Requirements:

- Rust via rustup with the MSVC toolchain (the exact version is pinned in
  `rust-toolchain.toml` and installed automatically);
- Visual Studio Build Tools with the "Desktop development with C++" workload;
- Node.js LTS (24.x);
- WebView2 runtime (preinstalled on Windows 11).

```powershell
npm ci
npm run tauri dev      # development mode with hot reload
npm run tauri build    # NSIS installer and .msi in src-tauri/target/release/bundle/
```

Running and debugging the GUI is only possible natively.

## Checks

```powershell
npm run check                                   # svelte-check
npm run lint                                    # eslint
cd src-tauri
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

End-to-end tests against a real SSH server (skipped unless configured):

```powershell
$env:TERMNEST_TEST_TARGET = "user@127.0.0.1:2222"
$env:TERMNEST_TEST_KEY = "C:\path\to\private_key"
$env:TERMNEST_TEST_PASSWORD = "..."   # only for the key deploy test
cargo test -p termnest-core --test live_ssh
```

A disposable server: `docker run -d -p 127.0.0.1:2222:2222 -e PUBLIC_KEY="<public key>"
-e USER_NAME=tester -e PASSWORD_ACCESS=true -e USER_PASSWORD=<password>
linuxserver/openssh-server`.
