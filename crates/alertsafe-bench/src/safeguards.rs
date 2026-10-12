//! H6: comparison with Compression Safeguards (Tyree et al., 2026), the
//! closest related work. The safeguards are Python-only, so the experiment is
//! split in three steps:
//!
//! 1. `alertsafe-bench export DIR` writes every dataset (timestamps, values,
//!    rules, and the window lengths in samples that the rules actually read);
//! 2. `baselines/safeguards/run.py DIR` translates the rules into safeguards,
//!    compresses with SZ3 alone and with SZ3 wrapped in the safeguards over a
//!    sweep of SZ3 error bounds, and writes every reconstruction back;
//! 3. `alertsafe-bench h6 DIR` checks every reconstruction with the reference
//!    rule semantics and compares the sizes with the rule-aware codec.
//!
//! Files are raw little-endian `i64`/`f64` arrays, a hand-written JSON
//! description for Python, and a whitespace-separated table for Rust, to keep
//! the bench free of dependencies.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use alertsafe_codec::{alert_diff, budget, compress};
use alertsafe_rules::{Agg, Cmp, Rule};

use crate::gen::Dataset;
use crate::relax::{relaxed, RELAX_KAPPA};
use crate::{datasets, eval_grid, tuned, uniform_sweep, value_range, EVAL_MS, EVAL_OFFSET_MS};

fn agg_name(a: Agg) -> &'static str {
    match a {
        Agg::Last => "last",
        Agg::AvgOverTime => "avg_over_time",
        Agg::SumOverTime => "sum_over_time",
        Agg::MinOverTime => "min_over_time",
        Agg::MaxOverTime => "max_over_time",
        Agg::Rate => "rate",
    }
}

fn cmp_name(c: Cmp) -> &'static str {
    match c {
        Cmp::Gt => ">",
        Cmp::Ge => ">=",
        Cmp::Lt => "<",
        Cmp::Le => "<=",
    }
}

/// Distinct support lengths (in samples) over the instants the codec
/// protects: every evaluation phase for `last`/`*_over_time` (Lemma 2), the
/// given grid for `rate`. Returns `[full, head, tail]`: interior windows, and
/// windows truncated by the start or the end of the series, which occur at
/// one position each. A stencil per full length, plus the truncated windows
/// where they occur, protects exactly the windows the rule can read: the most
/// favourable translation for the baseline.
fn support_lengths(rule: &Rule, ds: &Dataset, grid: &[i64]) -> [Vec<usize>; 3] {
    let n = ds.series.len();
    let inst = budget::instants(rule, &ds.series, grid);
    let mut sets: [BTreeSet<usize>; 3] = Default::default();
    for r in rule.supports(&ds.series.ts, &inst) {
        let k = match (r.start, r.end) {
            (s, e) if s == e => continue,
            (0, _) => 1,
            (_, e) if e == n => 2,
            _ => 0,
        };
        sets[k].insert(r.len());
    }
    sets.map(|s| s.into_iter().collect())
}

fn json_list(xs: &[usize]) -> String {
    xs.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_raw<T: Copy>(path: &Path, xs: &[T], to_le: impl Fn(T) -> [u8; 8]) {
    let bytes: Vec<u8> = xs.iter().flat_map(|&x| to_le(x)).collect();
    fs::write(path, bytes).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
}

fn read_f64(path: &Path) -> Vec<f64> {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    bytes
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

pub fn export(dir: &Path, days: f64) {
    for ds in &datasets(days) {
        let d = dir.join(ds.name);
        fs::create_dir_all(&d).unwrap();
        let s = &ds.series;
        let g = eval_grid(s);
        write_raw(&d.join("ts.i64"), &s.ts, i64::to_le_bytes);
        write_raw(&d.join("vals.f64"), &s.vals, f64::to_le_bytes);
        let mut json = String::new();
        write!(
            json,
            "{{\"name\": \"{}\", \"n\": {}, \"range\": {:e}, \"eval_ms\": {EVAL_MS}, \"eval_offset_ms\": {EVAL_OFFSET_MS}, \"rules\": [",
            ds.name,
            s.len(),
            value_range(s)
        )
        .unwrap();
        for (k, r) in ds.rules.iter().enumerate() {
            let [full, head, tail] = support_lengths(r, ds, &g);
            write!(
                json,
                "{}\n  {{\"name\": \"{}\", \"agg\": \"{}\", \"range_ms\": {}, \"cmp\": \"{}\", \"threshold\": {:e}, \"for_ms\": {}, \"keep_firing_for_ms\": {}, \"window_lengths\": [{}], \"head_lengths\": [{}], \"tail_lengths\": [{}]}}",
                if k > 0 { "," } else { "" },
                r.name,
                agg_name(r.agg),
                r.range_ms,
                cmp_name(r.cmp),
                r.threshold,
                r.for_ms,
                r.keep_firing_for_ms,
                json_list(&full),
                json_list(&head),
                json_list(&tail)
            )
            .unwrap();
        }
        json.push_str("\n]}\n");
        fs::write(d.join("meta.json"), json).unwrap();
        eprintln!(
            "exported {} ({} samples) to {}",
            ds.name,
            s.len(),
            d.display()
        );
    }
}

/// One row of `sweep.tsv` written by `run.py`.
struct Point {
    k: usize,
    eps: f64,
    sz3_bytes: usize,
    sg_bytes: usize,
    sg_secs: f64,
}

fn read_sweep(path: &Path) -> Vec<Point> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e} (run baselines/safeguards/run.py first)",
            path.display()
        )
    });
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Point {
                k: f[0].parse().unwrap(),
                eps: f[1].parse().unwrap(),
                sz3_bytes: f[2].parse().unwrap(),
                sg_bytes: f[3].parse().unwrap(),
                sg_secs: f[4].parse().unwrap(),
            }
        })
        .collect()
}

pub fn h6(dir: &Path, days: f64) {
    println!("\n## H6: Compression Safeguards baseline ({days} days)\n");
    println!("`SZ3` is SZ3 with an absolute error bound; `SZ3+SG` wraps it in safeguards translated from the rules");
    println!("(see `baselines/safeguards/run.py`); both are swept over the SZ3 bound. `tuned` = largest bound with no");
    println!("changed decision and none below it; `SZ3+SG best` = smallest output with zero state mismatches, if any.");
    println!("`rule-aware κ` is the optimistic budget with a posteriori repair (H7).");
    println!("Mismatches are counted by the reference evaluator of this repository.\n");
    println!("| dataset | lossless (ours) | uniform-tuned (ours) | rule-aware (ours) | rule-aware κ={RELAX_KAPPA} (ours) | SZ3 tuned | SZ3+SG best | SZ3+SG ε / range | SZ3+SG points with mismatches | SZ3+SG encode, s (mean) | ×SZ3+SG |");
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut sweeps = Vec::new();
    for ds in &datasets(days) {
        let d = dir.join(ds.name);
        let s = &ds.series;
        assert_eq!(
            read_f64(&d.join("vals.f64")),
            s.vals,
            "{}: export is stale, rerun export with the same days",
            ds.name
        );
        let g = eval_grid(s);
        let n = s.len() as f64;
        let range = value_range(s);
        let c = compress(s, &ds.rules, &g);
        let mode = c.plan.mode;
        let ours_tuned = tuned(&uniform_sweep(s, &ds.rules, &g, mode)).bits_per_value;
        let ours_lossless =
            alertsafe_codec::encode(&s.vals, &vec![0.0; s.len()], mode).bits_per_value();

        let mismatches = |file: String| {
            let recon = s.with_values(read_f64(&d.join("sweep").join(file)));
            assert_eq!(recon.len(), s.len());
            alert_diff(s, &recon, &ds.rules, &g).state_mismatches
        };
        let mut pts = read_sweep(&d.join("sweep.tsv"));
        pts.sort_by(|a, b| a.eps.total_cmp(&b.eps));
        let sz3_mm: Vec<usize> = pts
            .iter()
            .map(|p| mismatches(format!("{}_sz3.f64", p.k)))
            .collect();
        let sg_mm: Vec<usize> = pts
            .iter()
            .map(|p| mismatches(format!("{}_sg.f64", p.k)))
            .collect();
        let mut sweep = format!(
            "\n<details><summary>SZ3 / SZ3+SG sweep, <code>{}</code></summary>\n\n| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |\n|---:|---:|---:|---:|---:|---:|\n",
            ds.name
        );
        for (i, p) in pts.iter().enumerate() {
            writeln!(
                sweep,
                "| {:.1e} | {:.2} | {} | {:.2} | {} | {:.1} |",
                p.eps / range,
                p.sz3_bytes as f64 * 8.0 / n,
                sz3_mm[i],
                p.sg_bytes as f64 * 8.0 / n,
                sg_mm[i],
                p.sg_secs
            )
            .unwrap();
        }
        sweep.push_str("\n</details>");
        sweeps.push(sweep);

        let sz3_ok = sz3_mm.iter().position(|&m| m > 0).unwrap_or(pts.len());
        let sz3_tuned = if sz3_ok == 0 {
            "–".to_string()
        } else {
            format!("{:.2}", pts[sz3_ok - 1].sz3_bytes as f64 * 8.0 / n)
        };
        let best = (0..pts.len())
            .filter(|&i| sg_mm[i] == 0)
            .min_by_key(|&i| pts[i].sg_bytes);
        let sg_failed = sg_mm.iter().filter(|&&m| m > 0).count();
        let (relaxed_bits, _) = relaxed(ds, RELAX_KAPPA);
        let secs = pts.iter().map(|p| p.sg_secs).sum::<f64>() / pts.len().max(1) as f64;
        let (best_bits, best_eps, gain) = match best {
            Some(i) => {
                let b = pts[i].sg_bytes as f64 * 8.0 / n;
                (
                    format!("{b:.2}"),
                    format!("{:.1e}", pts[i].eps / range),
                    format!("{:.1}", b / relaxed_bits),
                )
            }
            None => ("–".into(), "–".into(), "–".into()),
        };
        println!(
            "| {} | {ours_lossless:.2} | {ours_tuned:.2} | {:.2} | {relaxed_bits:.2} | {sz3_tuned} | {best_bits} | {best_eps} | {sg_failed}/{} | {secs:.1} | {gain} |",
            ds.name,
            c.bits_per_value(),
            pts.len(),
        );
    }
    println!(
        "\nValues are bits per value (timestamps excluded). `×SZ3+SG` = SZ3+SG best / rule-aware κ={RELAX_KAPPA}."
    );
    for sweep in sweeps {
        println!("{sweep}");
    }
}
