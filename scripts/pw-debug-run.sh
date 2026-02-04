#!/usr/bin/env bash
set -euo pipefail

# Run from repo root: /home/sandor/Documents/codex/SwaySettings
export SWAYSETTINGS_PW_DEBUG=volume
export RUST_LOG=swaysettings::services::pipewire::audio_daemon=info

./target/debug/swaysettings |& rg "VOL-DBG"
