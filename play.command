#!/bin/zsh
# Dwarf Kingdom launcher — double-click me in Finder, or run ./play.command
cd "$(dirname "$0")"
source "$HOME/.cargo/env" 2>/dev/null
exec cargo run --release -p dk_app
