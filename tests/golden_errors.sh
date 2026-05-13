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
export CLAIMS_INTEGRITY_KEY_FILE="$TMP/integrity.key"
printf '%s\n' '1111111111111111111111111111111111111111111111111111111111111111' > "$CLAIMS_INTEGRITY_KEY_FILE"

# fixture: one pending claim we can try to verify
cd "$TMP"
"$CLMS" add "fixture claim for golden error tests" > /dev/null 2>&1
CLAIM_ID=$("$CLMS" timeline 2>/dev/null | awk '{print $1}' | head -1 | tr -d '#')

cat > "$TMP/stat_ok.json" <<'JSON'
{"test_type":"kolmogorov-smirnov","p_value":0.003,"sample_size":4821,"data_source":"real"}
JSON
cat > "$TMP/stat_missing_ds.json" <<'JSON'
{"test_type":"kolmogorov-smirnov","p_value":0.003,"sample_size":4821}
JSON
cat > "$TMP/bench_low.json" <<'JSON'
{"metric":"auc-roc","metric_value":0.6,"sample_size":1000,"data_source":"real"}
JSON
cat > "$TMP/bench_high_rmse.json" <<'JSON'
{"metric":"rmse","metric_value":0.9,"sample_size":1000,"data_source":"real"}
JSON
cat > "$TMP/bench_non_numeric.json" <<'JSON'
{"metric":"f1","metric_value":"NaN","sample_size":100,"data_source":"real"}
JSON
cat > "$TMP/bench_ok.json" <<'JSON'
{"metric":"f1","metric_value":0.8,"sample_size":100,"data_source":"real"}
JSON
cat > "$TMP/est_point_out.json" <<'JSON'
{"estimator":"mean","point_value":2.0,"ci_lower":0.9,"ci_upper":1.1,"confidence_level":0.95,"sample_size":100,"data_source":"real"}
JSON
cat > "$TMP/est_inverted_ci.json" <<'JSON'
{"estimator":"mean","point_value":1.0,"ci_lower":1.5,"ci_upper":0.5,"confidence_level":0.95,"sample_size":100,"data_source":"real"}
JSON
cat > "$TMP/est_conf_high.json" <<'JSON'
{"estimator":"mean","point_value":1.0,"ci_lower":0.9,"ci_upper":1.1,"confidence_level":1.5,"sample_size":100,"data_source":"real"}
JSON
cat > "$TMP/est_conf_zero.json" <<'JSON'
{"estimator":"mean","point_value":1.0,"ci_lower":0.9,"ci_upper":1.1,"confidence_level":0,"sample_size":100,"data_source":"real"}
JSON
cat > "$TMP/est_non_numeric_point.json" <<'JSON'
{"estimator":"mean","point_value":"Infinity","ci_lower":0.9,"ci_upper":1.1,"confidence_level":0.95,"sample_size":100,"data_source":"real"}
JSON

# normalize paths so the golden output is reproducible across runs and machines.
# - strip REPO_ROOT so the clms binary shows up as 'target/release/clms'
# - collapse mktemp tempdirs (macOS /var/folders/... and linux /tmp/...) to TMPDIR
normalize() {
  sed -e "s|$REPO_ROOT/||g" -e "s|/private$TMP|TMPDIR|g" -e "s|$TMP|TMPDIR|g" \
      -e 's|/var/folders/[A-Za-z0-9_/+]*/T/tmp\.[A-Za-z0-9]*|TMPDIR|g' \
      -e 's|\(stored:     \)[0-9a-f][0-9a-f]*|\1HASH|g' \
      -e 's|\(recomputed: \)[0-9a-f][0-9a-f]*|\1HASH|g'
}

run_case() {
  local label="$1"; shift
  echo "=== $label ==="
  echo "+ $*" | normalize
  set +e
  out=$("$@" 2>&1)
  code=$?
  set -e
  echo "exit=$code"
  echo "$out" | normalize
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

# --- unknown test_type on stat-test refused by clap's value-enum parser ---
run_case "stat-test: unknown test_type 'AUC'" \
  "$CLMS" verify "$CLAIM_ID" --method=stat-test --ref="x" --test-type=AUC --p-value=0.01 --sample-size=100 --data-source=real

run_case "stat-test: unknown test_type 'my-cool-test'" \
  "$CLMS" verify "$CLAIM_ID" --method=stat-test --ref="x" --test-type=my-cool-test --p-value=0.01 --sample-size=100 --data-source=real

# --- benchmark method ---
# unknown metric
run_case "benchmark: unknown metric 'NDCG'" \
  "$CLMS" verify "$CLAIM_ID" --method=benchmark --ref="x" --metric=NDCG --metric-value=0.5 --threshold=0.3 --sample-size=100 --data-source=real --cmd="echo"

# higher-better metric: value below threshold refused
run_case "benchmark: AUC 0.6 below threshold 0.8 (higher-better)" \
  "$CLMS" verify "$CLAIM_ID" --method=benchmark --ref="$TMP/bench_low.json" --threshold=0.8 --cmd="true"

# lower-better metric: value above threshold refused
run_case "benchmark: RMSE 0.9 above threshold 0.3 (lower-better)" \
  "$CLMS" verify "$CLAIM_ID" --method=benchmark --ref="$TMP/bench_high_rmse.json" --threshold=0.3 --cmd="true"

# non-finite metric_value
run_case "benchmark: metric_value not numeric in artifact" \
  "$CLMS" verify "$CLAIM_ID" --method=benchmark --ref="$TMP/bench_non_numeric.json" --threshold=0.5 --cmd="true"

# missing --threshold
run_case "benchmark: missing --threshold" \
  "$CLMS" verify "$CLAIM_ID" --method=benchmark --ref="$TMP/bench_ok.json" --cmd="true"

# --metric on prop-test (exclusive-flag violation)
run_case "benchmark: --metric on prop-test" \
  "$CLMS" verify "$CLAIM_ID" --method=prop-test --ref="x" --cmd="echo" --exit-code=0 --metric=f1

# --- estimate method ---
# unknown estimator
run_case "estimate: unknown estimator 'GaussianMode'" \
  "$CLMS" verify "$CLAIM_ID" --method=estimate --ref="x" --estimator=GaussianMode --point-value=1.0 --ci-lower=0.9 --ci-upper=1.1 --confidence-level=0.95 --sample-size=100 --data-source=real --cmd="echo"

# point outside CI
run_case "estimate: point outside CI" \
  "$CLMS" verify "$CLAIM_ID" --method=estimate --ref="$TMP/est_point_out.json" --cmd="true"

# ci_lower > ci_upper
run_case "estimate: ci_lower > ci_upper" \
  "$CLMS" verify "$CLAIM_ID" --method=estimate --ref="$TMP/est_inverted_ci.json" --cmd="true"

# confidence_level out of range
run_case "estimate: confidence_level=1.5 (out of range)" \
  "$CLMS" verify "$CLAIM_ID" --method=estimate --ref="$TMP/est_conf_high.json" --cmd="true"

# confidence_level=0 (boundary)
run_case "estimate: confidence_level=0 (boundary)" \
  "$CLMS" verify "$CLAIM_ID" --method=estimate --ref="$TMP/est_conf_zero.json" --cmd="true"

# non-finite point
run_case "estimate: point_value not numeric in artifact" \
  "$CLMS" verify "$CLAIM_ID" --method=estimate --ref="$TMP/est_non_numeric_point.json" --cmd="true"

# --estimator on observed (exclusive-flag violation)
run_case "estimate: --estimator on observed" \
  "$CLMS" verify "$CLAIM_ID" --method=observed --ref="x" --estimator=mean

# --- missing required field per method ---
run_case "prop-test missing --cmd" \
  "$CLMS" verify "$CLAIM_ID" --method=prop-test --exit-code=0 --ref="x"

run_case "integration-test missing --target" \
  "$CLMS" verify "$CLAIM_ID" --method=integration-test --cmd="echo" --exit-code=0 --ref="x"

run_case "replay-test missing --dataset" \
  "$CLMS" verify "$CLAIM_ID" --method=replay-test --cmd="echo" --exit-code=0 --ref="x"

run_case "stat-test missing --cmd" \
  "$CLMS" verify "$CLAIM_ID" --method=stat-test --ref="$TMP/stat_ok.json"

run_case "stat-test artifact missing data_source" \
  "$CLMS" verify "$CLAIM_ID" --method=stat-test --ref="$TMP/stat_missing_ds.json" --cmd="true"

run_case "documented missing --quote" \
  "$CLMS" verify "$CLAIM_ID" --method=documented --ref="x"

run_case "observed missing --ref" \
  "$CLMS" verify "$CLAIM_ID" --method=observed

run_case "derived missing --from" \
  "$CLMS" verify "$CLAIM_ID" --method=derived --ref="x"

# --- min_tier gate ---
# invalid tier name on add
run_case "min_tier: invalid tier name" \
  "$CLMS" add "claim with bogus tier" --min-tier=epirical

# helper: add claim & capture its newly-assigned seq (largest seq in timeline)
new_seq() {
  "$CLMS" timeline 2>/dev/null | awk '{print $1}' | tr -d '#' | sort -n | tail -1
}

# claim w/ min_tier=empirical, then verify with observed (using a real file ref
# so the bare-string-ref check doesn't fire first and mask the min_tier gate)
"$CLMS" add "science claim, demands empirical" --min-tier=empirical > /dev/null 2>&1
MIN_TIER_ID=$(new_seq)
echo "artifact body" > "$TMP/artifact.txt"

run_case "min_tier: refuse observed against empirical floor" \
  "$CLMS" verify "$MIN_TIER_ID" --method=observed --ref="$TMP/artifact.txt"

run_case "min_tier: refuse documented against empirical floor" \
  "$CLMS" verify "$MIN_TIER_ID" --method=documented --ref="https://docs.example.com" --quote="the api returns 200"

# claim w/ min_tier=observed, then refuse documented (which is below observed)
"$CLMS" add "observed-floor claim" --min-tier=observed > /dev/null 2>&1
OBS_TIER_ID=$(new_seq)

run_case "min_tier: refuse documented against observed floor" \
  "$CLMS" verify "$OBS_TIER_ID" --method=documented --ref="https://docs.example.com" --quote="text"

# --- repair mode is truly read-only ---
run_case "repair mode: add refused" \
  env CLAIMS_REPAIR=1 CLAIMS_INTEGRITY_KEY_FILE="$CLAIMS_INTEGRITY_KEY_FILE" "$CLMS" add "should refuse"

run_case "repair mode: reindex refused" \
  env CLAIMS_REPAIR=1 CLAIMS_INTEGRITY_KEY_FILE="$CLAIMS_INTEGRITY_KEY_FILE" "$CLMS" reindex

run_case "repair mode: migrate-integrity refused" \
  env CLAIMS_REPAIR=1 CLAIMS_INTEGRITY_KEY_FILE="$CLAIMS_INTEGRITY_KEY_FILE" "$CLMS" migrate-integrity

# strict-only integrity: hash-only claims are refused even if the old
# .integrity.strict marker is absent.
python3 - <<'PY'
import json
p = '.claims/000001.json'
obj = json.load(open(p))
obj['integrity_mac'] = None
open(p, 'w').write(json.dumps(obj, indent=2) + '\n')
PY
rm -f .claims/.integrity.strict

run_case "integrity: hash-only claim refused" \
  "$CLMS" show 1

run_case "integrity: migrate hash-only claim" \
  "$CLMS" migrate-integrity

# migration refuses to sign a legacy hash-only claim if its content_hash no
# longer matches.
BAD_LEDGER="$TMP/bad-ledger"
export BAD_LEDGER
mkdir -p "$BAD_LEDGER"
"$CLMS" --dir "$BAD_LEDGER" add "bad legacy claim" > /dev/null 2>&1
python3 - <<'PY'
import json, os
p = os.environ['BAD_LEDGER'] + '/.claims/000001.json'
obj = json.load(open(p))
obj['integrity_mac'] = None
obj['text'] = 'tampered legacy claim'
open(p, 'w').write(json.dumps(obj, indent=2) + '\n')
PY

run_case "integrity: migrate refuses content_hash mismatch" \
  "$CLMS" --dir "$BAD_LEDGER" migrate-integrity

# tamper only the keyed MAC: content_hash still verifies, so read_claim must
# fail on integrity_mac specifically.
python3 - <<'PY'
import json
p = '.claims/000001.json'
obj = json.load(open(p))
obj['integrity_mac'] = '0' * 64
open(p, 'w').write(json.dumps(obj, indent=2) + '\n')
PY

run_case "integrity: tampered integrity_mac refused" \
  "$CLMS" show 1
