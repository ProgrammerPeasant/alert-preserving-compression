#!/usr/bin/env bash
# Runs the Kani harnesses of alertsafe-codec (crates/alertsafe-codec/src/proofs.rs).
# Self-contained: rustup, the Kani toolchain and CBMC are installed under
# ./.kani-env, nothing is written to $HOME. Needs curl, a C linker (cc) and
# internet access on the first run.
#
#   scripts/kani/run.sh                  # all harnesses
#   JOBS=4 scripts/kani/run.sh           # parallel harnesses (default: RAM/8 GB)
#   TIMEOUT=8h scripts/kani/run.sh theorem1_avg_over_time_gt
#
# Results: kani-results/<harness>.log and kani-results/summary.txt.
set -euo pipefail

KANI_VERSION=0.68.0
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ENV_DIR=$ROOT/.kani-env
OUT=$ROOT/kani-results
export RUSTUP_HOME=$ENV_DIR/rustup CARGO_HOME=$ENV_DIR/cargo KANI_HOME=$ENV_DIR/kani
export PATH=$CARGO_HOME/bin:$PATH CARGO_TARGET_DIR=$ENV_DIR/target

# Longest first, so that the slow floating-point proofs start immediately.
ALL=(
  theorem1_{avg,sum}_over_time_{gt,ge,lt,le}
  scalar_tolerance_{gt,ge,lt,le} quantize_matches_decoder_and_budget
  theorem1_last_{gt,ge,lt,le}
  extremum_lipschitz_{last,max_over_time,min_over_time}
  extremum_attained_{max,min}_over_time extremum_pair_{max,min} sub_monotone
  tolerance_of_extremum_{last,max_over_time,min_over_time}
  range_coder_{bit,bits3,raw8,raw16,raw20,mixed}
  bit_writer_matches_spec bit_reader_matches_spec bit_spec_roundtrip
  varint_roundtrip zigzag_roundtrip
)
HARNESSES=("${@:-${ALL[@]}}")
mem_gb=$(awk '/MemAvailable/ { print int($2 / 1048576) }' /proc/meminfo)
JOBS=${JOBS:-$(( mem_gb / 8 > 0 ? mem_gb / 8 : 1 ))}
JOBS=$(( JOBS > $(nproc) ? $(nproc) : JOBS ))
TIMEOUT=${TIMEOUT:-6h}

if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
  echo "== installing rustup into $ENV_DIR"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
    sh -s -- -y --no-modify-path --profile minimal --default-toolchain stable
fi
if [ ! -x "$CARGO_HOME/bin/cargo-kani" ]; then
  echo "== installing kani-verifier $KANI_VERSION"
  cargo install --locked kani-verifier --version "$KANI_VERSION"
fi
if [ ! -d "$KANI_HOME/kani-$KANI_VERSION" ]; then
  echo "== kani setup (downloads CBMC and the pinned nightly)"
  cargo kani setup
fi

cd "$ROOT"
mkdir -p "$OUT"
echo "== compiling harnesses"
cargo kani -p alertsafe-codec --only-codegen > "$OUT/codegen.log" 2>&1 ||
  { tail -30 "$OUT/codegen.log"; exit 1; }

echo "== ${#HARNESSES[@]} harnesses, $JOBS in parallel, timeout $TIMEOUT each"
echo "   (progress: ls $OUT; tail -f $OUT/<harness>.log)"
run_one() {
  local h=$1 start=$SECONDS status
  if timeout "$TIMEOUT" cargo kani -p alertsafe-codec --harness "proofs::$h" --exact \
      > "$OUT/$h.log" 2>&1; then
    status=SUCCESSFUL
  elif [ $? -eq 124 ]; then
    status=TIMEOUT
  elif grep -q "VERIFICATION:- FAILED" "$OUT/$h.log"; then
    status=FAILED
  else
    status=ERROR
  fi
  printf '%-40s %-10s %6ds\n' "$h" "$status" $(( SECONDS - start )) | tee -a "$OUT/summary.txt"
}
export -f run_one
export OUT TIMEOUT
: > "$OUT/summary.txt"
printf '%s\n' "${HARNESSES[@]}" | xargs -P "$JOBS" -I{} bash -c 'run_one {}'

{
  echo
  echo "kani $KANI_VERSION, $(nproc) cores, ${mem_gb} GB available, jobs=$JOBS, $(date -Is)"
  grep -h "Failed Checks" "$OUT"/*.log | sort | uniq -c || true
} >> "$OUT/summary.txt"
echo "== done"
cat "$OUT/summary.txt"
echo "Send back: tar czf kani-results.tar.gz kani-results"
