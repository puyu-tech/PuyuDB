#!/bin/sh
# Follow-up measurements: Turso and 256 KiB chunks. macOS only.
set -eu
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/adr02-storage-target}"
cargo build --release --workspace
bins="adr02-rusqlite adr02-ffi adr02-libsql adr02-redb adr02-fjall adr02-heed adr02-turso"
echo "==== settings ===="
for bin in adr02-rusqlite adr02-turso; do
  dir=$(mktemp -d "/tmp/adr02-settings-${bin}.XXXX")
  echo "-- $bin"
  "$CARGO_TARGET_DIR/release/$bin" --cmd settings --dir "$dir"
  rm -rf "$dir"
done
echo "==== small turso ===="
dir=$(mktemp -d /tmp/adr02-turso-small.XXXX)
"$CARGO_TARGET_DIR/release/adr02-turso" --cmd measure --dir "$dir" --count 300 --bytes 64 --seed 1
echo "du_kib $(du -sk "$dir" | awk '{print $1}')"
rm -rf "$dir"
echo "==== 256KiB chunks count=40 ===="
for bin in $bins; do
  dir=$(mktemp -d "/tmp/adr02-large-${bin}.XXXX")
  echo "-- $bin"
  "$CARGO_TARGET_DIR/release/$bin" --cmd measure --dir "$dir" --count 40 --bytes 262144 --seed 3
  echo "du_kib $(du -sk "$dir" | awk '{print $1}')"
  rm -rf "$dir"
done
echo "==== fullfsync spot check count=80 bytes=64 ===="
export ADR02_FULLFSYNC=1
for bin in adr02-rusqlite adr02-turso; do
  dir=$(mktemp -d "/tmp/adr02-ffs-${bin}.XXXX")
  echo "-- $bin"
  "$CARGO_TARGET_DIR/release/$bin" --cmd measure --dir "$dir" --count 80 --bytes 64 --seed 4
  rm -rf "$dir"
done
