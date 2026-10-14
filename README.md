# termnest

Personal SSH client for Windows 10/11 built with Tauri 2, Rust and Svelte.
Key-based authentication only, low memory footprint, local storage.

Full specification: [SPEC.md](SPEC.md).

## Status

Implemented stage by stage (see SPEC.md, section 9). Current: stage 1 (skeleton):
one window, one terminal panel, connect by private key to `user@host:port`,
resize, copy and paste.

## Project layout

```
src/                    Svelte + TypeScript frontend (xterm.js terminal)
src-tauri/              Tauri app crate: thin command layer and events
src-tauri/core/         termnest-core: SSH sessions and other UI-independent logic
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
