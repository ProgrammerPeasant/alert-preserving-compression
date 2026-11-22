#!/usr/bin/env bash
# Runs the Kani harnesses of alertsafe-codec (crates/alertsafe-codec/src/proofs.rs).
# Self-contained: rustup, the Kani toolchain and CBMC are installed under
# ./.kani-env, nothing is written to $HOME. Needs curl, perl, a C linker (cc;
# on macOS: xcode-select --install) and internet access on the first run.
# Linux x86_64 and macOS (Intel / Apple Silicon).
#
#   scripts/kani/run.sh                  # all harnesses
#   JOBS=4 scripts/kani/run.sh           # parallel harnesses (default: RAM/8 GB)
#   TIMEOUT=8h scripts/kani/run.sh theorem1_avg_over_time_gt
#   scripts/kani/run.sh paper           # only the harnesses the preprint cites
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
  scalar_tolerance_{gt,ge,lt,le} quantize_step_{gauge,counter}
  theorem1_last_{gt,ge,lt,le}
  extremum_lipschitz_last
  extremum_attained_{max,min}_over_time extremum_pair_{max,min} sub_monotone
  eval_monotone_{last,max_over_time,min_over_time}
  tolerance_of_extremum_{last,max_over_time,min_over_time}
  range_coder_{bit,bits3,raw8,raw16,raw20,mixed}
  bit_writer_matches_spec bit_reader_matches_spec bit_spec_roundtrip
  varint_roundtrip zigzag_roundtrip
)
# Proved results cited in paper/sections/verification.tex.
PAPER=(
  quantize_step_{gauge,counter}
  extremum_pair_{max,min} extremum_attained_{max,min}_over_time sub_monotone
  scalar_tolerance_{gt,ge,lt,le}
  tolerance_of_extremum_{last,max_over_time,min_over_time}
  extremum_lipschitz_last
  eval_monotone_{last,max_over_time,min_over_time}
  bit_spec_roundtrip varint_roundtrip zigzag_roundtrip
)
if [ "${1:-}" = paper ]; then set -- "${PAPER[@]}"; fi
HARNESSES=("${@:-${ALL[@]}}")
# Linux and macOS (bash 3.2, no GNU coreutils needed).
if [ -r /proc/meminfo ]; then
  mem_gb=$(awk '/MemAvailable/ { print int($2 / 1048576) }' /proc/meminfo)
else
  mem_gb=$(( $(sysctl -n hw.memsize) / 1073741824 ))
fi
cores=$(getconf _NPROCESSORS_ONLN)
JOBS=${JOBS:-$(( mem_gb / 8 > 0 ? mem_gb / 8 : 1 ))}
JOBS=$(( JOBS > cores ? cores : JOBS ))
TIMEOUT=${TIMEOUT:-6h}
case $TIMEOUT in
  *h) TIMEOUT_S=$(( ${TIMEOUT%h} * 3600 )) ;;
  *m) TIMEOUT_S=$(( ${TIMEOUT%m} * 60 )) ;;
  *s) TIMEOUT_S=${TIMEOUT%s} ;;
  *)  TIMEOUT_S=$TIMEOUT ;;
esac
# Runs "$@" in its own process group and kills the whole group (cargo-kani
# and its cbmc child) after TIMEOUT_S seconds; exit status 124 on timeout.
with_timeout() {
  perl -e '
    my $t = shift; my $pid = fork;
    if ($pid == 0) { setpgrp(0, 0); exec @ARGV or exit 127 }
    $SIG{ALRM} = sub { kill "TERM", -$pid; sleep 10; kill "KILL", -$pid; exit 124 };
    alarm $t; waitpid($pid, 0);
    exit($? & 127 ? 128 + ($? & 127) : $? >> 8);
  ' "$TIMEOUT_S" "$@"
}

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
  if with_timeout cargo kani -p alertsafe-codec --harness "proofs::$h" --exact \
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
export -f run_one with_timeout
export OUT TIMEOUT_S
: > "$OUT/summary.txt"
printf '%s\n' "${HARNESSES[@]}" | xargs -P "$JOBS" -I{} bash -c 'run_one {}'

{
  echo
  echo "kani $KANI_VERSION, ${cores} cores, ${mem_gb} GB, $(uname -sm), jobs=$JOBS, $(date '+%Y-%m-%dT%H:%M:%S%z')"
  grep -h "Failed Checks" "$OUT"/*.log | sort | uniq -c || true
} >> "$OUT/summary.txt"
echo "== done"
cat "$OUT/summary.txt"
echo "Send back: COPYFILE_DISABLE=1 tar czf kani-results.tar.gz kani-results run.log"
