#!/usr/bin/env bash
# Build the UTA Test Gain VST3 fixture and assemble a .vst3 bundle.
# Output: target/test-plugins/bundle/UTA Test Gain.vst3
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --release --manifest-path "$ROOT/fixtures/test-plugins/uta-test-gain/Cargo.toml" --target-dir "$ROOT/target/test-plugins"
OUT="$ROOT/target/test-plugins/bundle/UTA Test Gain.vst3"
case "$(uname -s)" in
  Linux) ARCH=x86_64-linux; SRC=libuta_test_gain.so; EXT=so ;;
  MINGW*|MSYS*|CYGWIN*) ARCH=x86_64-win; SRC=uta_test_gain.dll; EXT=vst3 ;;
  *) echo "unsupported platform"; exit 1 ;;
esac
mkdir -p "$OUT/Contents/$ARCH"
cp "$ROOT/target/test-plugins/release/$SRC" "$OUT/Contents/$ARCH/UTA Test Gain.$EXT"
echo "$OUT"
