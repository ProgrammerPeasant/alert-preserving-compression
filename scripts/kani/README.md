# Kani proofs: running on a compute server

Requirements: Linux x86_64, `bash`, `curl`, a C toolchain (`cc`), internet
access on the first run, ~5 GB of disk. Nothing is installed outside this
directory (`.kani-env/`). Each floating-point harness can take 20 min to
several hours and several GB of RAM; the script runs `RAM / 8 GB` harnesses
at once by default.

```sh
tar xzf alertsafe-kani.tar.gz && cd alertsafe-kani
nohup scripts/kani/run.sh > run.log 2>&1 &     # or inside tmux/screen
# when finished:
tar czf kani-results.tar.gz kani-results
```

Knobs: `JOBS=4` (parallel harnesses), `TIMEOUT=12h` (per harness),
harness names as arguments to run a subset. `kani-results/summary.txt` lists
SUCCESSFUL / FAILED / TIMEOUT per harness; a FAILED log contains the failing
check. Concrete counterexample values:
`cargo kani -p alertsafe-codec --harness proofs::<name> --exact -Z concrete-playback --concrete-playback=print`
(with the environment from the top of `run.sh`).
