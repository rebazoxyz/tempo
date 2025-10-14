#!/usr/bin/env python3
"""
Run the tempo max-TPS bench workload, shut down the node, and optionally analyze logs.

Python port of the legacy bench_and_kill.sh helper so the benchmark pipeline can stay in Python.
"""

from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
from pathlib import Path

# Resolve repository root from scripts/ directory.
REPO_ROOT = Path(__file__).resolve().parents[2]
ANALYZE_SCRIPT = REPO_ROOT / "analyze_log.py"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run tempo bench workload and stop the node.")
    parser.add_argument(
        "--log",
        default=os.environ.get("TEMPO_LOG_FILE", "debug.log"),
        help="Path to debug log produced by the tempo node (default: TEMPO_LOG_FILE or debug.log)",
    )
    parser.add_argument(
        "--json-output",
        dest="json_output",
        help="Write summary metrics JSON to the given path",
    )
    parser.add_argument(
        "--label",
        help="Label to include in the metrics JSON",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="Suppress verbose analysis output",
    )
    parser.add_argument(
        "--skip-analysis",
        action="store_true",
        help="Skip the log analysis step (use when orchestrator handles it)",
    )
    parser.add_argument(
        "--duration-seconds",
        type=int,
        default=180,
        help="Duration to run tempo-bench before stopping (default: 180 seconds)",
    )
    return parser.parse_args()


def run_tempo_bench(duration_seconds: int) -> None:
    print("Step 1: Running tempo-bench with max-tps...")
    cmd = [
        "cargo",
        "run",
        "--bin",
        "tempo-bench",
        "run-max-tps",
        "--duration",
        str(duration_seconds),
        "--tps",
        "20000",
        "--target-urls",
        "http://localhost:8545",
        "--disable-thread-pinning",
        "true",
        "--chain-id",
        "1337",
    ]
    process = subprocess.Popen(cmd, cwd=REPO_ROOT)
    # Add buffer time for tempo-bench to finish cleanly after its internal duration expires
    timeout = duration_seconds + 30
    try:
        process.wait(timeout=timeout)
        if process.returncode not in (0, None):
            raise subprocess.CalledProcessError(process.returncode, cmd)
        print(f"tempo-bench completed successfully.")
        return
    except subprocess.TimeoutExpired:
        print(f"tempo-bench still running after {timeout}s timeout, stopping workload...")
        process.send_signal(signal.SIGINT)
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            print("tempo-bench did not exit after SIGINT, killing process.")
            process.kill()
            process.wait()
    if process.returncode not in (0, None):
        raise subprocess.CalledProcessError(process.returncode or -1, cmd)
    print("tempo-bench stopped successfully.")


def find_tempo_pids() -> list[int]:
    print("\nStep 2: Finding tempo node process...")
    result = subprocess.run(
        ["pgrep", "-x", "tempo"],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 or not result.stdout.strip():
        return []
    pids: list[int] = []
    for token in result.stdout.split():
        try:
            pids.append(int(token))
        except ValueError:
            continue
    return pids


def kill_tempo(pids: list[int]) -> None:
    if not pids:
        raise RuntimeError("No tempo process found")

    print(f"Found tempo process IDs: {', '.join(map(str, pids))}")
    print("\nStep 3: Killing tempo node...")
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            continue

    for pid in pids:
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass
        except OSError:
            pass

    print("Tempo process killed successfully")


def analyze_logs(args: argparse.Namespace) -> None:
    if args.skip_analysis:
        print("\nSkipping log analysis step (requested via --skip-analysis).")
        return

    log_path = Path(args.log)
    print(f"\nStep 4: Analyzing logs ({log_path})...")

    analyze_args = [
        sys.executable,
        str(ANALYZE_SCRIPT),
        "--log",
        str(log_path),
    ]

    if args.json_output:
        json_path = Path(args.json_output)
        json_path.parent.mkdir(parents=True, exist_ok=True)
        analyze_args.extend(["--json", str(json_path)])

    if args.label:
        analyze_args.extend(["--label", args.label])

    if args.quiet:
        analyze_args.append("--quiet")

    subprocess.run(analyze_args, cwd=REPO_ROOT, check=True)


def main() -> None:
    args = parse_args()

    run_tempo_bench(args.duration_seconds)

    pids = find_tempo_pids()
    if not pids:
        raise SystemExit("No tempo process found")

    kill_tempo(pids)

    analyze_logs(args)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as err:
        raise SystemExit(err.returncode) from err
