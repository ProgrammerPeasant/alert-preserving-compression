//! H8: real Prometheus series (see `scripts/fetch_real.py`).
//!
//! The series are raw samples from the public Prometheus demo servers, and
//! the rules are the node-mixin / prometheus-mixin alerts those servers run,
//! restricted to one series (constant denominators folded into θ). Upstream
//! thresholds almost never fire on a healthy host, which is the common case
//! in production but makes the budget trivially large. Every series is
//! therefore also run with θ recalibrated to quantiles of the rule's own
//! aggregate on the evaluation grid:
//!
//! * `prod` — upstream thresholds, as deployed;
//! * `q95`  — θ at the 95th percentile (5th for `<`, `<=`): the condition holds
//!   about 5% of the time, a busy but plausible alert;
//! * `q50`  — θ at the median: the aggregate crosses θ all the time, the worst
//!   case for any decision-preserving codec.
//!
//! Counters without an upstream rule get `rate(x[5m]) > θ for 5m` and
//! `rate(x[5m]) < θ for 10m`, calibrated only.

use std::fs;
use std::path::Path;

use alertsafe_rules::{Agg, Cmp, Rule, Series};

use crate::gen::Dataset;
use crate::relax::{relaxed, RELAX_KAPPA};
use crate::safeguards::read_f64;
use crate::{eval_grid, summarize, PROM_CHUNK};

const MIN: i64 = 60_000;

/// One fetched series with its upstream rules (possibly none).
pub struct RealSeries {
    pub name: String,
    pub kind: String,
    pub source: String,
    pub series: Series,
    pub rules: Vec<Rule>,
}

fn read_i64(path: &Path) -> Vec<i64> {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&b| i64::from_le_bytes(b))
        .collect()
}

fn parse_agg(s: &str) -> Agg {
    match s {
        "last" => Agg::Last,
        "avg_over_time" => Agg::AvgOverTime,
        "sum_over_time" => Agg::SumOverTime,
        "min_over_time" => Agg::MinOverTime,
        "max_over_time" => Agg::MaxOverTime,
        "rate" => Agg::Rate,
        _ => panic!("unknown aggregate {s}"),
    }
}

fn parse_cmp(s: &str) -> Cmp {
    match s {
        ">" => Cmp::Gt,
        ">=" => Cmp::Ge,
        "<" => Cmp::Lt,
        "<=" => Cmp::Le,
        _ => panic!("unknown comparison {s}"),
    }
}

fn cmp_str(c: Cmp) -> &'static str {
    match c {
        Cmp::Gt => ">",
        Cmp::Ge => ">=",
        Cmp::Lt => "<",
        Cmp::Le => "<=",
    }
}

/// Reads `manifest.txt` and the raw arrays it names.
pub fn load(dir: &Path) -> Vec<RealSeries> {
    let path = dir.join("manifest.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e} (run scripts/fetch_real.py)",
            path.display()
        )
    });
    let mut out: Vec<RealSeries> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.first() {
            Some(&"series") => {
                let name = f[1].to_string();
                let ts = read_i64(&dir.join(format!("{name}.ts")));
                let vals = read_f64(&dir.join(format!("{name}.vals")));
                out.push(RealSeries {
                    name,
                    kind: f[2].to_string(),
                    source: f[3].to_string(),
                    series: Series::new(ts, vals),
                    rules: Vec::new(),
                });
            }
            Some(&"rule") => {
                let secs = |s: &str| s.parse::<i64>().unwrap() * 1000;
                let rule = Rule::new(
                    f[1],
                    parse_agg(f[2]),
                    secs(f[3]),
                    parse_cmp(f[4]),
                    f[5].parse().unwrap(),
                )
                .hold(secs(f[6]))
                .keep_firing(secs(f[7]));
                out.last_mut().expect("rule before series").rules.push(rule);
            }
            _ => {}
        }
    }
    out
}

/// `q`-quantile (nearest rank) of the rule's aggregate over the grid, moved
/// to the midpoint between it and the next distinct value. Metrics are often
/// discrete (load with two decimals, integer counter increments), so a raw
/// quantile usually equals some evaluations exactly; such ties leave no room
/// for any lossy codec and would say nothing about the method.
fn quantile(rule: &Rule, s: &Series, grid: &[i64], q: f64) -> f64 {
    let mut v: Vec<f64> = rule
        .eval_many(&s.ts, &s.vals, grid)
        .into_iter()
        .flatten()
        .filter(|x| x.is_finite())
        .collect();
    v.sort_by(f64::total_cmp);
    let a = v[((v.len() - 1) as f64 * q).round() as usize];
    match v.iter().find(|&&x| x > a) {
        Some(&b) => a + (b - a) / 2.0,
        None => match v.iter().rev().find(|&&x| x < a) {
            Some(&b) => b + (a - b) / 2.0,
            None => a,
        },
    }
}

/// Rules with θ moved to the `q`-quantile of each rule's aggregate (the
/// `1 - q` quantile for `<` and `<=`, so the condition holds equally often).
fn calibrated(rules: &[Rule], s: &Series, q: f64) -> Vec<Rule> {
    let g = eval_grid(s);
    rules
        .iter()
        .map(|r| {
            let q = match r.cmp {
                Cmp::Gt | Cmp::Ge => q,
                Cmp::Lt | Cmp::Le => 1.0 - q,
            };
            Rule {
                threshold: quantile(r, s, &g, q),
                ..r.clone()
            }
        })
        .collect()
}

pub const VARIANTS: [&str; 3] = ["prod", "q95", "q50"];

/// Every (series, variant) pair as a bench dataset named `series@variant`.
pub fn datasets(real: &[RealSeries]) -> Vec<Dataset> {
    let mut out = Vec::new();
    for r in real {
        let base = if r.rules.is_empty() {
            vec![
                Rule::new("RateHigh", Agg::Rate, 5 * MIN, Cmp::Gt, 0.0).hold(5 * MIN),
                Rule::new("RateLow", Agg::Rate, 5 * MIN, Cmp::Lt, 0.0).hold(10 * MIN),
            ]
        } else {
            r.rules.clone()
        };
        for v in VARIANTS {
            let rules = match v {
                "prod" if r.rules.is_empty() => continue,
                "prod" => base.clone(),
                "q95" => calibrated(&base, &r.series, 0.95),
                _ => calibrated(&base, &r.series, 0.5),
            };
            out.push(Dataset {
                name: Box::leak(format!("{}@{v}", r.name).into_boxed_str()),
                series: r.series.clone(),
                rules,
            });
        }
    }
    out
}

pub fn h8(dir: &Path) {
    let real = load(dir);
    let sets = datasets(&real);
    let days = real
        .iter()
        .map(|r| (r.series.ts[r.series.len() - 1] - r.series.ts[0]) as f64 / 86_400_000.0)
        .fold(0.0, f64::max);
    println!(
        "\n## H8: real Prometheus series ({} series, {days:.1} days, raw 15s scrapes)\n",
        real.len()
    );
    println!(
        "Sources: `prom-demo` = prometheus.demo.prometheus.io (node_exporter, Prometheus, Caddy),"
    );
    println!("`promlabs` = demo.promlabs.com (demo service). Rules: `prod` = upstream node-/prometheus-mixin");
    println!("thresholds restricted to one series; `q95` / `q50` = θ recalibrated to the 95th percentile / median");
    println!("of the rule's aggregate (see `real.rs`). Same columns as H1, plus H7 (κ={RELAX_KAPPA}) and");
    println!("Prometheus-sized chunks of {PROM_CHUNK}. Zero changed decisions in every rule-aware cell (asserted).\n");
    println!("| series | kind | rules | firings | lossless | uniform-safe | uniform-tuned | rule-aware | κ={RELAX_KAPPA} | rule-aware /{PROM_CHUNK} | κ /{PROM_CHUNK} | ×lossless | ×tuned | ×tuned (κ) | exact samples |");
    println!("|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut totals = std::collections::BTreeMap::<&str, [f64; 7]>::new();
    for ds in &sets {
        let (name, variant) = ds.name.split_once('@').unwrap();
        let src = real.iter().find(|r| r.name == name).unwrap();
        let kind = format!("{} ({})", src.kind, src.source);
        let r = summarize(ds, false);
        let (kappa, _) = relaxed(ds, RELAX_KAPPA, usize::MAX);
        let (chunk, _) = relaxed(ds, 1.0, PROM_CHUNK);
        let (kchunk, _) = relaxed(ds, RELAX_KAPPA, PROM_CHUNK);
        println!(
            "| {name} | {kind} | {variant} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {kappa:.2} | {chunk:.2} | {kchunk:.2} | {:.1} | {:.1} | {:.1} | {:.2}% |",
            r.episodes,
            r.lossless,
            r.uniform_safe,
            r.uniform_tuned,
            r.rule_aware,
            r.lossless / r.rule_aware,
            r.uniform_tuned / r.rule_aware,
            r.uniform_tuned / kappa,
            100.0 * r.exact_share,
        );
        let t = totals.entry(variant).or_default();
        for (a, b) in t.iter_mut().zip([
            r.lossless,
            r.uniform_safe,
            r.uniform_tuned,
            r.rule_aware,
            kappa,
            chunk,
            kchunk,
        ]) {
            *a += b;
        }
    }
    println!("\n<details><summary>Thresholds</summary>\n");
    println!("| series | rules | rule | expression | for | firings |");
    println!("|---|---|---|---|---:|---:|");
    for ds in &sets {
        let (name, variant) = ds.name.split_once('@').unwrap();
        let g = eval_grid(&ds.series);
        for r in &ds.rules {
            let w = if r.agg == Agg::Last {
                String::new()
            } else {
                format!("[{}m]", r.range_ms / MIN)
            };
            let firings = alertsafe_rules::episodes(&g, &r.trajectory(&ds.series, &g)).len();
            println!(
                "| {name} | {variant} | {} | `{:?}(x{w}) {} {:e}` | {}m | {firings} |",
                r.name,
                r.agg,
                cmp_str(r.cmp),
                r.threshold,
                r.for_ms / MIN
            );
        }
    }
    println!("\n</details>");
    println!("\nMean bits per value over the series of each rule variant (every series has the same length):\n");
    println!("| rules | series | lossless | uniform-safe | uniform-tuned | rule-aware | κ={RELAX_KAPPA} | rule-aware /{PROM_CHUNK} | κ /{PROM_CHUNK} |");
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for v in VARIANTS {
        let n = sets
            .iter()
            .filter(|d| d.name.ends_with(&format!("@{v}")))
            .count();
        if let Some(t) = totals.get(v) {
            let m: Vec<String> = t.iter().map(|x| format!("{:.2}", x / n as f64)).collect();
            println!("| {v} | {n} | {} |", m.join(" | "));
        }
    }
}
