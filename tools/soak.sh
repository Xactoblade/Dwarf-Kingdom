#!/usr/bin/env bash
# Dwarf Kingdom — long unattended soak run, for the Forge.
#
# Builds in --release and runs the #[ignore]d crash/soak guard
# (stress_play.rs::a_fort_survives_a_long_unattended_watch): 4 seeds x ~400k
# ticks each, invasions on, a living world behind it — the exact per-tick path
# the app runs. A panic prints its site with RUST_BACKTRACE=1.
#
# Designed to be launched DETACHED (see the nohup line at the bottom of the
# "how to run" note) so it survives the session that started it. Writes a live
# log and a one-line machine-readable verdict other tools/agents can grep.
#
# Usage:
#   tools/soak.sh                 # run the soak, log to saves/soak-<ts>.log
#   RUST_BACKTRACE=full tools/soak.sh
#
# Result markers written to the log (grep these):
#   SOAK-START <iso8601>
#   SOAK-RESULT pass|fail  seeds=4  ticks=400000  elapsed=<s>s  exit=<n>

set -u
cd "$(dirname "$0")/.." || exit 3
REPO="$(pwd)"

# cargo lives under rustup; make sure it's on PATH for a detached shell.
# shellcheck disable=SC1090
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

TS="$(date +%Y%m%d-%H%M%S)"
LOG="${SOAK_LOG:-$REPO/saves/soak-$TS.log}"
mkdir -p "$(dirname "$LOG")"

export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"

{
  echo "SOAK-START $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host=$(hostname)  cores=$(sysctl -n hw.ncpu 2>/dev/null || nproc)  repo=$REPO"
  echo "commit=$(git rev-parse --short HEAD 2>/dev/null)  branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)"
  echo "cargo=$(cargo --version 2>/dev/null)"
  echo "---- build (release) ----"
} >>"$LOG" 2>&1

START=$(date +%s)

# Build first so the build time isn't counted as soak hang, and so a compile
# error fails fast with a clear marker.
if ! cargo build -p dk_agents --release --tests >>"$LOG" 2>&1; then
  ELAPSED=$(( $(date +%s) - START ))
  echo "SOAK-RESULT fail  seeds=4  ticks=400000  elapsed=${ELAPSED}s  exit=build-error" >>"$LOG"
  exit 1
fi

{
  echo "---- soak (this is the long part; ~tens of minutes) ----"
} >>"$LOG" 2>&1

# The actual soak. --nocapture streams progress; the test is #[ignore]d so it
# only runs with --ignored.
cargo test -p dk_agents --release --test stress_play \
  -- --ignored --nocapture --exact \
  a_fort_survives_a_long_unattended_watch >>"$LOG" 2>&1
EXIT=$?

ELAPSED=$(( $(date +%s) - START ))
if [ "$EXIT" -eq 0 ]; then
  echo "SOAK-RESULT pass  seeds=4  ticks=400000  elapsed=${ELAPSED}s  exit=0" >>"$LOG"
else
  echo "SOAK-RESULT fail  seeds=4  ticks=400000  elapsed=${ELAPSED}s  exit=$EXIT" >>"$LOG"
fi
exit "$EXIT"
