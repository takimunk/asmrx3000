#!/usr/bin/env bash
# Run audio-state regression checks without opening an audio device or requiring XCTest.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if ! command -v cargo >/dev/null && [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi
cargo build --locked --release --manifest-path "$ROOT/core/Cargo.toml"
CHECK_DIR="$(mktemp -d -t asmr-checks)"
trap 'rm -rf "$CHECK_DIR"' EXIT
swiftc -parse-as-library -g \
  -I "$ROOT/apps/macos/Sources/CAsmr" "$ROOT/core/target/release/libasmr_core.a" \
  "$ROOT/apps/macos/Sources/ASMRX3000/AudioEngine.swift" \
  "$ROOT/apps/macos/Sources/ASMRX3000/HeadTracker.swift" \
  "$ROOT/apps/macos/Sources/ASMRX3000/Scenes.swift" \
  "$ROOT/apps/macos/Sources/ASMRX3000/Onboarding/OnboardingState.swift" \
  "$ROOT/apps/macos/Tests/AudioStateCheck.swift" \
  -o "$CHECK_DIR/audio-state-check"
"$CHECK_DIR/audio-state-check"
