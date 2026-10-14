#!/bin/sh
# Builds the Windows release inside the container and copies the results to ./release.
set -eu
cd /src

TARGET=x86_64-pc-windows-msvc
OUT="src-tauri/target/${TARGET}/release"

npm ci --no-audit --no-fund
npm run tauri build -- --runner cargo-xwin --target "${TARGET}" --bundles nsis

mkdir -p release
rm -f release/*.exe
cp "${OUT}/termnest.exe" release/termnest.exe
cp "${OUT}"/bundle/nsis/*-setup.exe release/

echo "Build results:"
ls -l release
