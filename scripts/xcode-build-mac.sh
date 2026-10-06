#!/bin/bash
# Xcode external build for the existing Tauri macOS app.
# Opens from macos/UnitAgent.xcodeproj. Does not replace the Rust application.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "The Unit Agent Xcode project produces the macOS .app and .dmg. Open it on a Mac." >&2
  exit 1
fi

if [[ -z "${UNIT_AGENT_CARGO_TARGET:-}" && -n "${ARCHS:-}" ]]; then
  has_arm=0
  has_x64=0
  for arch in $ARCHS; do
    if [[ "$arch" == "arm64" ]]; then has_arm=1; fi
    if [[ "$arch" == "x86_64" ]]; then has_x64=1; fi
  done
  if [[ $has_arm -eq 1 && $has_x64 -eq 1 ]]; then
    export UNIT_AGENT_CARGO_TARGET=universal-apple-darwin
  elif [[ $has_arm -eq 1 ]]; then
    export UNIT_AGENT_CARGO_TARGET=aarch64-apple-darwin
  elif [[ $has_x64 -eq 1 ]]; then
    export UNIT_AGENT_CARGO_TARGET=x86_64-apple-darwin
  fi
fi

npm run build:mac
