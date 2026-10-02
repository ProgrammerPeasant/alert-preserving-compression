//! Synthetic experiments for H1 (gain over uniform error), H2 (gain vs. time
//! spent away from thresholds) and H3 (codec throughput).
//!
//! Usage: `alertsafe-bench [h1|h2|h3|all] [days]`

mod gen;

use std::time::Instant;

use alertsafe_codec::{
    alert_diff, budget, compress, decode, encode, encode_with, Coding, Encoded, Mode,
};
use alertsafe_rules::{grid, Rule, Series};
use gen::Dataset;

/// Rule evaluation interval; the grid is deliberately out of phase with scrapes.
const EVAL_MS: i64 = 30_000;
const EVAL_OFFSET_MS: i64 = 7_000;

fn eval_grid(s: &Series) -> Vec<i64> {
    grid(s.ts[0] + EVAL_OFFSET_MS, s.ts[s.len() - 1], EVAL_MS)
}

fn roundtrip(s: &Series, budget: &[f64], mode: Mode) -> (Encoded, Series) {
    let enc = encode(&s.vals, budget, mode);
    let recon = s.with_values(decode(&enc.bytes));
    (enc, recon)
}

fn value_range(s: &Series) -> f64 {
    let (lo, hi) = s
        .vals
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    (hi - lo).max(f64::MIN_POSITIVE)
}

struct Sweep {
    eps: f64,
    bits_per_value: f64,
    mismatches: usize,
    changed: usize,
    max_shift_s: f64,
}

/// Uniform error bound `eps` swept over powers of four of the value range.
fn uniform_sweep(s: &Series, rules: &[Rule], g: &[i64], mode: Mode) -> Vec<Sweep> {
    let range = value_range(s);
    (0..=18)
        .map(|k| {
            let eps = range * 4f64.powi(k - 18);
            let (enc, recon) = roundtrip(s, &vec![eps; s.len()], mode);
            let d = alert_diff(s, &recon, rules, g);
            Sweep {
                eps,
                bits_per_value: enc.bits_per_value(),
                mismatches: d.state_mismatches,
                changed: d.changed_firings(),
                max_shift_s: d.max_shift_ms as f64 / 1000.0,
            }
        })
        .collect()
}

/// Largest swept uniform bound such that it and every smaller one leave the
/// trajectories untouched: the best a user tuning a uniform bound on this
/// very data could do, without any guarantee on other data.
fn tuned(sweep: &[Sweep]) -> &Sweep {
    let k = sweep
        .iter()
        .position(|w| w.mismatches > 0)
        .unwrap_or(sweep.len());
    &sweep[k.saturating_sub(1)]
}

struct Summary {
    lossless: f64,
    uniform_safe: f64,
    uniform_tuned: f64,
    rule_aware: f64,
    /// Same plan, fixed-width residual codes (entropy-coding ablation).
    rule_aware_fixed: f64,
    predicted_gain_bits: f64,
    repairs: usize,
    exact_share: f64,
    episodes: usize,
}

fn summarize(ds: &Dataset, print_sweep: bool) -> Summary {
    let s = &ds.series;
    let g = eval_grid(s);
    let c = compress(s, &ds.rules, &g);
    let mode = c.plan.mode;
    let recon = s.with_values(decode(&c.encoded.bytes));
    let d = alert_diff(s, &recon, &ds.rules, &g);
    assert_eq!(
        d.state_mismatches, 0,
        "rule-aware codec changed a decision on {}",
        ds.name
    );

    let lossless = roundtrip(s, &vec![0.0; s.len()], mode).0;
    // Largest uniform bound satisfying the same sufficient condition. Samples
    // the plan keeps exact (counter resets, exact ties) stay exact here too.
    let raw_budget = budget::plan(s, &ds.rules, &g).budget;
    let eps_safe = raw_budget
        .iter()
        .copied()
        .filter(|&b| b > 0.0)
        .fold(f64::INFINITY, f64::min);
    let safe_budget: Vec<f64> = raw_budget
        .iter()
        .map(|&b| if b == 0.0 { 0.0 } else { eps_safe })
        .collect();
    let (safe_enc, safe_recon) = roundtrip(s, &safe_budget, mode);
    assert_eq!(
        alert_diff(s, &safe_recon, &ds.rules, &g).state_mismatches,
        0
    );

    // High-resolution quantization theory: the rate difference between two
    // quantizers is the mean log-ratio of their steps.
    let finite: Vec<f64> = c
        .plan
        .budget
        .iter()
        .copied()
        .filter(|&b| b > 0.0 && b.is_finite())
        .collect();
    let predicted_gain_bits = finite
        .iter()
        .map(|&b| (b.max(eps_safe) / eps_safe).log2())
        .sum::<f64>()
        / finite.len().max(1) as f64;

    let sweep = uniform_sweep(s, &ds.rules, &g, mode);
    if print_sweep {
        println!(
            "\n<details><summary>Uniform-bound sweep, <code>{}</code></summary>\n",
            ds.name
        );
        println!("| ε / range | bits/value | state mismatches | changed firings | max shift, s |");
        println!("|---:|---:|---:|---:|---:|");
        let range = value_range(s);
        for w in &sweep {
            println!(
                "| {:.1e} | {:.2} | {} | {} | {:.0} |",
                w.eps / range,
                w.bits_per_value,
                w.mismatches,
                w.changed,
                w.max_shift_s
            );
        }
        println!("\n</details>");
    }
    let episodes = ds
        .rules
        .iter()
        .map(|r| alertsafe_rules::episodes(&g, &r.trajectory(s, &g)).len())
        .sum();
    Summary {
        lossless: lossless.bits_per_value(),
        uniform_safe: safe_enc.bits_per_value(),
        uniform_tuned: tuned(&sweep).bits_per_value,
        rule_aware: c.encoded.bits_per_value(),
        rule_aware_fixed: encode_with(&s.vals, &c.plan.budget, mode, Coding::Fixed)
            .bits_per_value(),
        predicted_gain_bits,
        repairs: c.repairs,
        exact_share: c.plan.budget.iter().filter(|&&b| b == 0.0).count() as f64 / s.len() as f64,
        episodes,
    }
}

fn h1(days: f64) {
    println!(
        "## H1: rule-aware vs. uniform error bound ({days} days, 15s scrape, {}s eval)\n",
        EVAL_MS / 1000
    );
    println!("All rule-aware rows have zero changed decisions (asserted). `uniform-safe` is the largest uniform bound");
    println!("that satisfies the same sufficient condition; `uniform-tuned` is the largest uniform bound with zero");
    println!("state mismatches found by sweeping on the same data (no guarantee elsewhere).\n");
    println!("| dataset | firings | lossless | uniform-safe | uniform-tuned | rule-aware | rule-aware (fixed) | ×lossless | ×safe | ×tuned | predicted Δbits (safe→aware) | measured Δbits | exact samples | repairs |");
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let sets = [
        gen::cpu(1, days, 0.8, 1.5),
        gen::latency(2, days),
        gen::memory(3, days),
        gen::requests(4, days),
    ];
    let mut sweeps = Vec::new();
    for ds in &sets {
        let r = summarize(ds, false);
        println!(
            "| {} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.1} | {:.1} | {:.1} | {:.2} | {:.2} | {:.2}% | {} |",
            ds.name,
            r.episodes,
            r.lossless,
            r.uniform_safe,
            r.uniform_tuned,
            r.rule_aware,
            r.rule_aware_fixed,
            r.lossless / r.rule_aware,
            r.uniform_safe / r.rule_aware,
            r.uniform_tuned / r.rule_aware,
            r.predicted_gain_bits,
            r.uniform_safe - r.rule_aware,
            100.0 * r.exact_share,
            r.repairs
        );
        sweeps.push(ds);
    }
    println!("\nValues are bits per value (timestamps excluded). Raw float64 is 64 bits. All columns except\n`rule-aware (fixed)` use the entropy-coded format.");
    for ds in sweeps {
        summarize(ds, true);
    }
}

fn h2(days: f64) {
    println!(
        "\n## H2: gain vs. distance from the threshold (`cpu`, rule `HighCPU` only, {days} days)\n"
    );
    println!("`near` is the share of samples whose budget is below 0.01 (i.e. the 5m average is within 0.01 of θ).\n");
    println!("| θ | incidents/day | firings | near | lossless | uniform-tuned | rule-aware | ×lossless | ×tuned |");
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for &per_day in &[0.5, 2.0, 6.0] {
        for &th in &[0.55, 0.65, 0.75, 0.85, 0.95] {
            let mut ds = gen::cpu(7, days, th, per_day);
            ds.rules.truncate(1);
            let r = summarize(&ds, false);
            let near = budget::plan(&ds.series, &ds.rules, &eval_grid(&ds.series))
                .budget
                .iter()
                .filter(|&&b| b < 0.01)
                .count() as f64
                / ds.series.len() as f64;
            println!(
                "| {th} | {per_day} | {} | {:.1}% | {:.2} | {:.2} | {:.2} | {:.1} | {:.1} |",
                r.episodes,
                100.0 * near,
                r.lossless,
                r.uniform_tuned,
                r.rule_aware,
                r.lossless / r.rule_aware,
                r.uniform_tuned / r.rule_aware
            );
        }
    }
}

fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut out = None;
    for _ in 0..runs {
        let t = Instant::now();
        let v = f();
        best = best.min(t.elapsed().as_secs_f64());
        out = Some(v);
    }
    (best, out.unwrap())
}

fn h3(days: f64) {
    println!("\n## H3: single-core throughput (MB/s of raw float64 input, best of 5)\n");
    println!("| dataset | samples | budget (naive) | budget | encode (rule-aware) | decode | encode (fixed) | decode (fixed) | encode (lossless) | end-to-end `compress` |");
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for ds in [gen::cpu(1, days, 0.8, 1.5), gen::requests(4, days)] {
        let s = &ds.series;
        let g = eval_grid(s);
        let mb = s.len() as f64 * 8.0 / 1e6;
        let (t_naive, _) = best_of(5, || budget::plan_naive(s, &ds.rules, &g));
        let (t_plan, plan) = best_of(5, || budget::plan(s, &ds.rules, &g));
        let (t_enc, enc) = best_of(5, || encode(&s.vals, &plan.budget, plan.mode));
        let (t_dec, _) = best_of(5, || decode(&enc.bytes));
        let (t_enc_fx, enc_fx) = best_of(5, || {
            encode_with(&s.vals, &plan.budget, plan.mode, Coding::Fixed)
        });
        let (t_dec_fx, _) = best_of(5, || decode(&enc_fx.bytes));
        let zeros = vec![0.0; s.len()];
        let (t_ll, _) = best_of(5, || encode(&s.vals, &zeros, plan.mode));
        let (t_all, _) = best_of(5, || compress(s, &ds.rules, &g));
        println!(
            "| {} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
            ds.name,
            s.len(),
            mb / t_naive,
            mb / t_plan,
            mb / t_enc,
            mb / t_dec,
            mb / t_enc_fx,
            mb / t_dec_fx,
            mb / t_ll,
            mb / t_all
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let what = args.get(1).map(String::as_str).unwrap_or("all");
    let days: f64 = args.get(2).and_then(|d| d.parse().ok()).unwrap_or(7.0);
    match what {
        "h1" => h1(days),
        "h2" => h2(days),
        "h3" => h3(days),
        "all" => {
            h1(days);
            h2(days);
            h3(days);
        }
        other => eprintln!("unknown experiment {other}; expected h1, h2, h3 or all"),
    }
}
