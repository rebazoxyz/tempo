#!/bin/bash
# Bench both Reth revisions by switching Cargo git revs, running tempo, and summarizing metrics.

set -euo pipefail

FEATURE_COMMIT="1619408"
MAIN_COMMIT="d2070f4de34f523f6097ebc64fa9d63a04878055"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Background PID for tempo + tee pipeline; tracked so we can wait/kill safely.
TEMPO_PIPE_PID=""

# Kill tempo processes on script exit to avoid dangling nodes.
cleanup() {
  if pgrep -qx tempo >/dev/null 2>&1; then
    echo "Cleaning up tempo processes..."
    pkill -x tempo || true
  fi

  if [[ -n "${TEMPO_PIPE_PID}" ]]; then
    if ps -p "${TEMPO_PIPE_PID}" >/dev/null 2>&1; then
      kill "${TEMPO_PIPE_PID}" || true
    fi
    TEMPO_PIPE_PID=""
  fi
}
trap cleanup EXIT

# Swap the reth git revision, update dependencies, and rebuild tempo.
switch_commit() {
  local label="$1"
  local commit="$2"

  echo ""
  echo "=== Switching to ${label} commit ${commit} ==="
  sed -i '' 's/git = "https:\/\/github.com\/paradigmxyz\/reth", rev = "[^"]*"/git = "https:\/\/github.com\/paradigmxyz\/reth", rev = "'"${commit}"'"/g' "${SCRIPT_DIR}/Cargo.toml"

  echo ""
  echo "Updating reth dependency..."
  (cd "${SCRIPT_DIR}" && cargo update -p reth)

  echo ""
  echo "Building tempo (--release)..."
  (cd "${SCRIPT_DIR}" && cargo build --release)
}

start_tempo_node() {
  local log_file="$1"

  : > "${log_file}"
  echo ""
  echo "Starting tempo node..."
  (
    cd "${SCRIPT_DIR}"
    tempo node \
      --http \
      --http.addr 0.0.0.0 \
      --http.port 8545 \
      --http.api all \
      --datadir ./data \
      --dev \
      --dev.block-time 1s \
      --chain genesis.json \
      --engine.disable-precompile-cache \
      --builder.gaslimit 3000000000 \
      --builder.max-tasks 8 \
      --builder.deadline 4 \
      --txpool.pending-max-count 10000000000000 \
      --txpool.basefee-max-count 10000000000000 \
      --txpool.queued-max-count 10000000000000 \
      --txpool.pending-max-size 10000 \
      --txpool.basefee-max-size 10000 \
      --txpool.queued-max-size 10000 \
      --txpool.max-new-pending-txs-notifications 10000000 \
      --txpool.max-account-slots 500000 \
      --txpool.max-pending-txns 10000000000000 \
      --txpool.max-new-txns 10000000000000 \
      --txpool.disable-transactions-backup \
      --txpool.additional-validation-tasks 8 \
      --txpool.minimal-protocol-fee 0 \
      --txpool.minimum-priority-fee 0 \
      --rpc.max-connections 429496729 \
      --rpc.max-request-size 1000000 \
      --rpc.max-response-size 1000000 \
      --max-tx-reqs 1000000 2>&1 | tee >(rg "build_payload|Received block from consensus engine|State root task finished|Block added to canonical chain" > "${log_file}")
  ) &
  TEMPO_PIPE_PID=$!

  wait_for_tempo
}

# Poll the JSON-RPC endpoint until tempo responds.
wait_for_tempo() {
  echo "Waiting for tempo HTTP endpoint..."
  local payload='{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

  for _ in {1..60}; do
    if curl -s -f -H "Content-Type: application/json" --data "${payload}" http://localhost:8545 >/dev/null 2>&1; then
      echo "Tempo HTTP endpoint is ready."
      return 0
    fi
    sleep 1
  done

  echo "Tempo HTTP endpoint did not become ready in time." >&2
  exit 1
}

# Execute a full cycle: switch commit, run tempo, bench, harvest metrics.
run_bench_cycle() {
  local label="$1"
  local commit="$2"
  local log_file="$3"
  local metrics_file="$4"

  switch_commit "${label}" "${commit}"
  start_tempo_node "${log_file}"

  echo ""
  echo "Running bench_and_kill.sh for ${label}..."
  (
    cd "${SCRIPT_DIR}"
    ./bench_and_kill.sh \
      --log "${log_file}" \
      --json-output "${metrics_file}" \
      --label "${label}" \
      --quiet
  )

  echo ""
  echo "Waiting for tempo node to exit..."
  if [[ -n "${TEMPO_PIPE_PID}" ]]; then
    wait "${TEMPO_PIPE_PID}" || true
    TEMPO_PIPE_PID=""
  fi
  echo "Bench cycle for ${label} complete. Metrics saved to ${metrics_file}"
}

# Print a before/after comparison table when both metrics files exist.
print_comparison() {
  local before_file="$1"
  local after_file="$2"

  if [[ ! -f "${before_file}" || ! -f "${after_file}" ]]; then
    echo "Comparison skipped: missing metrics files."
    return
  fi

  python3 - "${before_file}" "${after_file}" <<'PY'
import json
import sys
from pathlib import Path

before_path, after_path = sys.argv[1], sys.argv[2]
before = json.loads(Path(before_path).read_text())
after = json.loads(Path(after_path).read_text())

metrics_order = [
    "Build Payload Time",
    "State Root Computation",
    "Explicit State Root Task",
    "Block Added to Canonical Chain",
]

stat_labels = [
    ("mean", "Average"),
    ("median", "Median"),
    ("min", "Min"),
    ("max", "Max"),
    ("std_dev", "Std Dev"),
]

def fmt(value):
    if value is None:
        return "n/a"
    return f"{value:.3f} ms"

def fmt_signed(value):
    if value is None:
        return "n/a"
    return f"{value:+.3f} ms"

def fmt_pct(before_val, diff):
    if before_val in (None, 0):
        return "n/a"
    return f"{(diff / before_val) * 100:+.1f}%"

print("Complete Metrics Comparison")
print("{:<28} {:<10} {:>14} {:>14} {:>14} {:>10}".format("Metric", "Statistic", "Before", "After", "Abs Diff", "% Change"))

for metric in metrics_order:
    before_stats = before["metrics"].get(metric)
    after_stats = after["metrics"].get(metric)

    for stat_key, stat_label in stat_labels:
        before_val = before_stats.get(stat_key) if before_stats else None
        after_val = after_stats.get(stat_key) if after_stats else None

        if before_val is None and after_val is None:
            continue

        diff = None
        if before_val is not None and after_val is not None:
            diff = after_val - before_val

        diff_str = fmt_signed(diff) if diff is not None else "n/a"
        pct_str = fmt_pct(before_val, diff) if diff is not None else "n/a"

        print(
            "{:<28} {:<10} {:>14} {:>14} {:>14} {:>10}".format(
                metric if stat_label == "Average" else "",
                stat_label,
                fmt(before_val),
                fmt(after_val),
                diff_str,
                pct_str,
            )
        )
PY
}

main() {
  local main_log="${SCRIPT_DIR}/debug_main.log"
  local feature_log="${SCRIPT_DIR}/debug_feature.log"
  local main_metrics="${SCRIPT_DIR}/metrics_main.json"
  local feature_metrics="${SCRIPT_DIR}/metrics_feature.json"

  echo "Starting main -> feature bench cycles..."
  run_bench_cycle "main" "${MAIN_COMMIT}" "${main_log}" "${main_metrics}"
  run_bench_cycle "feature" "${FEATURE_COMMIT}" "${feature_log}" "${feature_metrics}"
  echo ""
  echo "All bench cycles completed."
  echo ""
  print_comparison "${main_metrics}" "${feature_metrics}"
}

main "$@"
