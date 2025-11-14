#!/usr/bin/env bash
# Compare storage constants between current branch and another branch
#
# Usage:
#   ./scripts/compare_constants.sh <comparison-branch>
#
# Example:
#   ./scripts/compare_constants.sh feat/simplify-storage
#
# This script will:
# 1. Export constants from the current branch
# 2. Export constants from the comparison branch
# 3. Save a diff file showing differences
# 4. Display the diff to stdout

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Get script directory and workspace root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Function to print colored output
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_header() {
    echo -e "${BLUE}[====]${NC} $1"
}

# Check arguments
if [ $# -ne 1 ]; then
    print_error "Usage: $0 <comparison-branch>"
    print_error "Example: $0 feat/simplify-storage"
    print_error ""
    print_error "This will compare the current branch against the specified branch."
    exit 1
fi

COMPARISON_BRANCH="$1"

# Validate comparison branch exists
if ! git rev-parse --verify "$COMPARISON_BRANCH" >/dev/null 2>&1; then
    print_error "Branch '$COMPARISON_BRANCH' does not exist"
    exit 1
fi

# Get current branch
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
print_info "Current branch: $CURRENT_BRANCH"
print_info "Comparison branch: $COMPARISON_BRANCH"

# # Check for uncommitted changes
# if ! git diff-index --quiet HEAD --; then
#     print_warn "You have uncommitted changes. Please commit or stash them first."
#     exit 1
# fi

cd "$WORKSPACE_ROOT"

# Generate sanitized filenames
CURRENT_BRANCH_SAFE="${CURRENT_BRANCH//\//_}"
COMPARISON_BRANCH_SAFE="${COMPARISON_BRANCH//\//_}"

CURRENT_FILE="${CURRENT_BRANCH_SAFE}_constants.json"
COMPARISON_FILE="${COMPARISON_BRANCH_SAFE}_constants.json"
DIFF_FILE="${CURRENT_BRANCH_SAFE}_vs_${COMPARISON_BRANCH_SAFE}.diff"

# Function to export constants from a branch
export_from_branch() {
    local branch=$1
    local output_file=$2
    local is_current=$3

    if [ "$is_current" = "false" ]; then
        print_info "Switching to branch: $branch"
        git checkout "$branch" >/dev/null 2>&1
    fi

    print_info "Running export test for $branch..."
    cargo test --package tempo-precompiles export_all_storage_constants -- --ignored --nocapture 2>&1 | grep -E "(✅|Running|Compiling|Finished)" || true

    if [ -f "current_branch_constants.json" ]; then
        mv current_branch_constants.json "$output_file"
        print_info "Exported constants to: $output_file"
    else
        print_error "Failed to export constants from $branch"
        return 1
    fi
}

# Export from current branch first (no checkout needed)
print_header "Step 1: Exporting from current branch ($CURRENT_BRANCH)"
export_from_branch "$CURRENT_BRANCH" "$CURRENT_FILE" "true"

# Export from comparison branch
print_header "Step 2: Exporting from comparison branch ($COMPARISON_BRANCH)"
export_from_branch "$COMPARISON_BRANCH" "$COMPARISON_FILE" "false"

# Restore current branch
print_info "Restoring branch: $CURRENT_BRANCH"
git checkout "$CURRENT_BRANCH" >/dev/null 2>&1

# Compare the files
print_header "Step 3: Comparing branches"

# Use jq to sort and pretty-print for reliable diff
if command -v jq >/dev/null 2>&1; then
    print_info "Using jq for normalized comparison..."
    jq -S . "$CURRENT_FILE" > /tmp/current_sorted.json
    jq -S . "$COMPARISON_FILE" > /tmp/comparison_sorted.json

    # Generate diff
    if diff -u /tmp/current_sorted.json /tmp/comparison_sorted.json > "$DIFF_FILE" 2>&1; then
        print_info "${GREEN}✓ No differences found!${NC}"
        echo "No differences detected." > "$DIFF_FILE"
    else
        print_warn "Differences detected!"
    fi

    # Cleanup temp files
    rm -f /tmp/current_sorted.json /tmp/comparison_sorted.json
else
    print_warn "jq not found. Using simple diff (results may be less reliable)."
    if diff -u "$CURRENT_FILE" "$COMPARISON_FILE" > "$DIFF_FILE" 2>&1; then
        print_info "${GREEN}✓ No differences found!${NC}"
        echo "No differences detected." > "$DIFF_FILE"
    else
        print_warn "Differences detected!"
    fi
fi

# Display results
print_header "Results"
echo ""
print_info "Generated files:"
print_info "  📄 $CURRENT_FILE (current branch: $CURRENT_BRANCH)"
print_info "  📄 $COMPARISON_FILE (comparison branch: $COMPARISON_BRANCH)"
print_info "  📊 $DIFF_FILE (diff output)"
echo ""

# Show diff content
print_header "Diff Output"
echo ""
cat "$DIFF_FILE"
echo ""

# Summary
if grep -q "No differences detected" "$DIFF_FILE"; then
    print_info "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    print_info "${GREEN}✓ Storage layouts are identical!${NC}"
    print_info "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
else
    print_warn "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print_warn "⚠ Storage layout differences detected!"
    print_warn "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    print_info "Review the diff file for details: $DIFF_FILE"
fi
