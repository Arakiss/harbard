#!/usr/bin/env bash
# Measure warm end-to-end `harbard brief` latency (process spawn + work), the
# hook-path budget. Target: under 10 ms warm at p95.
#
# Usage: scripts/bench-brief.sh [iterations]
set -euo pipefail

ITER="${1:-300}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/harbard"
[ -x "$BIN" ] || { echo "build first: cargo build --release -p harbard-cli"; exit 1; }

HOME_DIR="$(mktemp -d)"
trap 'rm -rf "$HOME_DIR"' EXIT
export HARBARD_HOME="$HOME_DIR/store"
REPO="$HOME_DIR/repo"
mkdir -p "$REPO"

# Populate a realistic store: two other live sessions + a couple of notes.
echo '{"session_id":"codex-a","cwd":"'"$REPO"'"}' | "$BIN" hello --hook codex --intent "editing store.rs" >/dev/null 2>&1
echo '{"session_id":"codex-a","cwd":"'"$REPO"'"}' | "$BIN" note "watch the append path" --hook codex >/dev/null 2>&1
echo '{"session_id":"other-b","cwd":"'"$REPO"'"}' | "$BIN" hello --hook other --intent "reviewing" >/dev/null 2>&1

# The viewer session for the measured brief.
export CLAUDE_CODE_SESSION_ID=claude-bench

python3 - "$BIN" "$REPO" "$ITER" <<'PY'
import subprocess, sys, time
bin_, repo, iters = sys.argv[1], sys.argv[2], int(sys.argv[3])
cmd = [bin_, "brief", "--cwd", repo]
# Warmup (page cache, dynamic linker).
for _ in range(20):
    subprocess.run(cmd, capture_output=True)
samples = []
for _ in range(iters):
    t0 = time.perf_counter()
    subprocess.run(cmd, capture_output=True)
    samples.append((time.perf_counter() - t0) * 1000.0)
samples.sort()
def pct(p): return samples[min(len(samples)-1, int(len(samples)*p))]
p50, p95, p99, mx = pct(0.50), pct(0.95), pct(0.99), samples[-1]
print(f"iterations : {iters} (after 20 warmup)")
print(f"p50        : {p50:.3f} ms")
print(f"p95        : {p95:.3f} ms")
print(f"p99        : {p99:.3f} ms")
print(f"max        : {mx:.3f} ms")
budget = 10.0
verdict = "PASS" if p95 < budget else "FAIL"
print(f"budget     : < {budget:.0f} ms warm p95  ->  {verdict}")
sys.exit(0 if p95 < budget else 1)
PY
