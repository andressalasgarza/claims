#!/usr/bin/env bash
# golden_errors.sh — capture every invalid `clms` invocation's exit code +
# stderr verbatim. agents self-correct from these messages, so they MUST
# stay stable across refactors.
#
# usage:
#   tests/golden_errors.sh > tests/golden_errors.out
#   git diff tests/golden_errors.out  # MUST be empty after a refactor
#
# the test exercises:
#   1. refused methods (unit-test, code-test, sim-test) — schema 1.1 rejects
#   2. cross-flag rejections (--target only with integration-test, etc.)
#   3. --data-source=simulated rejected on stat-test
#   4. missing required fields per method
#   5. unknown method
#
# every case here is a parse-time or validate-time rejection. each MUST
# exit non-zero with a specific stderr message.

set -u
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLMS="${CLMS:-$REPO_ROOT/target/release/clms}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# fixture: one pending claim we can try to verify
cd "$TMP"
"$CLMS" add "fixture claim for golden error tests" > /dev/null 2>&1
CLAIM_ID=$("$CLMS" timeline 2>/dev/null | awk '{print $1}' | head -1 | tr -d '#')

run_case() {
  local label="$1"; shift
  echo "=== $label ==="
  echo "+ $*"
  set +e
  out=$("$@" 2>&1)
  code=$?
  set -e
  echo "exit=$code"
  echo "$out"
  echo
}

# --- refused methods (schema 1.1) ---
run_case "refused: unit-test" \
  "$CLMS" verify "$CLAIM_ID" --method=unit-test --cmd="echo" --exit-code=0 --ref="x"

run_case "refused: code-test" \
  "$CLMS" verify "$CLAIM_ID" --method=code-test --cmd="echo" --exit-code=0 --ref="x"

run_case "refused: sim-test" \
  "$CLMS" verify "$CLAIM_ID" --method=sim-test --cmd="echo" --exit-code=0 --ref="x"

# --- unknown method ---
run_case "unknown: bogus-method" \
  "$CLMS" verify "$CLAIM_ID" --method=bogus-method --cmd="echo" --exit-code=0 --ref="x"

# --- cross-flag rejection: --target only on integration-test ---
run_case "cross-flag: --target on prop-test" \
  "$CLMS" verify "$CLAIM_ID" --method=prop-test --cmd="echo" --exit-code=0 --ref="x" --target="https://api.example.com"

# --- cross-flag rejection: --dataset only on replay-test ---
run_case "cross-flag: --dataset on prop-test" \
  "$CLMS" verify "$CLAIM_ID" --method=prop-test --cmd="echo" --exit-code=0 --ref="x" --dataset="data.csv"

# --- cross-flag rejection: --data-source only on stat-test ---
run_case "cross-flag: --data-source on prop-test" \
  "$CLMS" verify "$CLAIM_ID" --method=prop-test --cmd="echo" --exit-code=0 --ref="x" --data-source=real

# --- simulated rejected on stat-test ---
run_case "simulated rejected on stat-test" \
  "$CLMS" verify "$CLAIM_ID" --method=stat-test --cmd="echo" --exit-code=0 --ref="x" --data-source=simulated

# --- missing required field per method ---
run_case "prop-test missing --cmd" \
  "$CLMS" verify "$CLAIM_ID" --method=prop-test --exit-code=0 --ref="x"

run_case "integration-test missing --target" \
  "$CLMS" verify "$CLAIM_ID" --method=integration-test --cmd="echo" --exit-code=0 --ref="x"

run_case "replay-test missing --dataset" \
  "$CLMS" verify "$CLAIM_ID" --method=replay-test --cmd="echo" --exit-code=0 --ref="x"

run_case "stat-test missing --data-source" \
  "$CLMS" verify "$CLAIM_ID" --method=stat-test --cmd="echo" --exit-code=0 --ref="x"

run_case "documented missing --quote" \
  "$CLMS" verify "$CLAIM_ID" --method=documented --ref="x"

run_case "observed missing --ref" \
  "$CLMS" verify "$CLAIM_ID" --method=observed

run_case "derived missing --from" \
  "$CLMS" verify "$CLAIM_ID" --method=derived --ref="x"
