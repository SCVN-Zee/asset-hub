#!/bin/sh
# Puts the release asset-hub-cli at crates/app/binaries/asset-hub-cli-<host triple>, where Tauri
# looks for the bundled `externalBin`.
#
# tauri-build (crates/app/build.rs) checks that file exists, and it runs for every cargo build of
# the package, the CLI's own build included. On a clean checkout the file does not exist yet, so an
# empty placeholder is created first. `cargo test` and `cargo clippy` need the placeholder too:
# run this script once (or `touch` the file) before them on a clean checkout.
set -eu

cd "$(dirname "$0")/.."

triple=$(rustc -vV | sed -n 's/^host: //p')
[ -n "$triple" ] || { echo "cli-sidecar: could not read the host triple from rustc" >&2; exit 1; }

dir=crates/app/binaries
sidecar="$dir/asset-hub-cli-$triple"

mkdir -p "$dir"
[ -e "$sidecar" ] || : > "$sidecar"

cargo build --manifest-path crates/app/Cargo.toml --release --bin asset-hub-cli

built=target/release/asset-hub-cli
[ -s "$built" ] || { echo "cli-sidecar: $built is missing or empty after the cargo run" >&2; exit 1; }

cp "$built" "$sidecar.tmp"
chmod +x "$sidecar.tmp"
mv "$sidecar.tmp" "$sidecar"
echo "cli-sidecar: $sidecar ($(wc -c < "$sidecar" | tr -d ' ') bytes)"
