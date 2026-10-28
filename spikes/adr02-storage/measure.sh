#!/bin/sh
# Reproduce the ADR-02 spike measurements. Not a workspace target.
set -eu
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/adr02-storage-target}"
cargo build --release --workspace
echo "==== platform ===="
uname -a
sw_vers
rustc --version
sysctl -n machdep.cpu.brand_string hw.ncpu hw.memsize
for bin in adr02-rusqlite adr02-ffi adr02-libsql adr02-redb adr02-fjall adr02-heed; do
  dir=$(mktemp -d "/tmp/adr02-measure-${bin}.XXXX")
  echo "==== $bin measure ===="
  "$CARGO_TARGET_DIR/release/$bin" --cmd measure --dir "$dir" --count 300 --seed 1
  echo "du_kib $(du -sk "$dir" | awk '{print $1}')"
  rm -rf "$dir"
  dir=$(mktemp -d "/tmp/adr02-recover-${bin}.XXXX")
  echo "==== $bin crash-recovery ===="
  "$CARGO_TARGET_DIR/release/$bin" --cmd loop --dir "$dir" --seed 7 --count 40 >"$dir/out" 2>"$dir/child.err" &
  pid=$!
  i=0
  while [ "$i" -lt 200 ]; do
    if grep -q "ACK 1" "$dir/out" 2>/dev/null; then
      break
    fi
    i=$((i + 1))
    sleep 0.05
  done
  sleep 0.02
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  /usr/bin/time -p "$CARGO_TARGET_DIR/release/$bin" --cmd verify --dir "$dir" --acks "$dir/acks.log"
  rm -rf "$dir"
done
