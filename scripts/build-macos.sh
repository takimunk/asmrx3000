#!/usr/bin/env bash
# Builds asmr-core (Rust) + the macOS menu bar app and assembles build/ASMR X3000.app.
#   scripts/build-macos.sh          build and launch
#   OPEN=0 scripts/build-macos.sh   build only
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_PKG="$ROOT/apps/macos"
APP="$ROOT/build/ASMR X3000.app"

# rustup installs into ~/.cargo; pick it up even if the shell profile was not updated.
if ! command -v cargo >/dev/null && [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

echo "▸ asmr-core (rust, release)"
cargo build --manifest-path "$ROOT/core/Cargo.toml" --release

echo "▸ macOS shell (swift, release)"
BIN_DIR="$(swift build --package-path "$APP_PKG" -c release --show-bin-path)"
# SwiftPM does not track the Rust static library, so force a relink.
rm -f "$BIN_DIR/ASMRX3000"
swift build --package-path "$APP_PKG" -c release

echo "▸ bundle"
pkill -x ASMRX3000 2>/dev/null || true
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN_DIR/ASMRX3000" "$APP/Contents/MacOS/ASMRX3000"
cp "$APP_PKG/Support/Info.plist" "$APP/Contents/Info.plist"
codesign --force --sign - "$APP"
echo "✓ $APP"

if [[ "${OPEN:-1}" == "1" ]]; then
  open "$APP"
fi
