#!/bin/bash
# Fetch the current epoch from the consensus metrics endpoint

METRICS_URL="${1:-http://127.0.0.1:8001/metrics}"

curl -s "$METRICS_URL" | grep -E "epoch_manager_latest_epoch" | grep -v "^#"
