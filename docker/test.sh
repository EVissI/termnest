#!/bin/sh
# Runs all checks inside the container: frontend type check and lint,
# Rust formatting, core unit tests and clippy for the Windows target.
set -eu
cd /src

npm ci --no-audit --no-fund
npm run check
npm run lint
# tauri::generate_context! requires the built frontend to exist.
npm run build

cd src-tauri
cargo fmt --all -- --check
cargo test -p termnest-core
cargo clippy -p termnest-core --all-targets -- -D warnings
cargo xwin clippy --target x86_64-pc-windows-msvc --workspace --all-targets -- -D warnings

echo "All checks passed."
