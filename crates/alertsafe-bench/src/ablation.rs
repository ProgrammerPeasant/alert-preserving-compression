//! Ablation of the codec's design choices on synthetic and real data.
//!
//! A cumulative ladder from the plain Theorem 1 codec to the full one, each
//! row adding one component, plus leave-one-out rows that remove one
//! component from the full codec:
//!
//! * box vs one-sided budget (Corollary 1, Proposition 1(e)),
//! * nearest-only vs adaptive (lazy) step choice,
//! * scale search ([`Options::scales`], off in the full codec),
//! * optimistic budget with repair ([`Options::slack`]),
//! * entropy vs fixed-width residual coding (on the full analytic plan).
//!
//! Every configuration is checked for zero changed decisions.

use std::thread;

use alertsafe_codec::{alert_diff, compress_with, encode_bounds, Coding, Options, Steps, SCALES};

use crate::gen::Dataset;
use crate::relax::RELAX_KAPPA;
use crate::{datasets, eval_grid};

struct Config {
    name: &'static str,
    opts: Options,
}

fn configs() -> Vec<Config> {
    let full = Options::default();
    let base = Options {
        one_sided: false,
        steps: Steps::Nearest,
        ..full
    };
    vec![
        Config {
            name: "box, nearest (Thm 1)",
            opts: base,
        },
        Config {
            name: "+ one-sided",
            opts: Options {
                one_sided: true,
                ..base
            },
        },
        Config {
            name: "+ lazy steps (= rule-aware)",
            opts: full,
        },
        Config {
            name: "+ κ=1024 (= κ)",
            opts: Options {
                slack: RELAX_KAPPA,
                ..full
            },
        },
        Config {
            name: "rule-aware + scale search",
            opts: Options {
                scales: SCALES,
                ..full
            },
        },
        Config {
            name: "box, nearest + scale search",
            opts: Options {
                scales: SCALES,
                ..base
            },
        },
        Config {
            name: "rule-aware − one-sided",
            opts: Options {
                one_sided: false,
                ..full
            },
        },
        Config {
            name: "rule-aware − lazy steps",
            opts: Options {
                steps: Steps::Nearest,
                ..full
            },
        },
        Config {
            name: "κ − one-sided",
            opts: Options {
                one_sided: false,
                slack: RELAX_KAPPA,
                ..full
            },
        },
    ]
}

/// Bits per value of every configuration, then of the full analytic plan
/// with fixed-width residual codes.
fn row(ds: &Dataset, cfgs: &[Config]) -> Vec<f64> {
    let s = &ds.series;
    let g = eval_grid(s);
    let mut out: Vec<f64> = cfgs
        .iter()
        .map(|c| {
            let r = compress_with(s, &ds.rules, &g, c.opts);
            let recon = s.with_values(r.decode());
            assert_eq!(
                alert_diff(s, &recon, &ds.rules, &g).state_mismatches,
                0,
                "{} changed a decision on {}",
                c.name,
                ds.name
            );
            r.bits_per_value()
        })
        .collect();
    let full = compress_with(s, &ds.rules, &g, Options::default());
    out.push(
        encode_bounds(
            &s.vals,
            &full.plan.down,
            &full.plan.up,
            full.plan.mode,
            Coding::Fixed,
            f64::NEG_INFINITY,
        )
        .bits_per_value(),
    );
    out
}

/// Rows of `sets` computed on `jobs` threads, in order.
fn rows(sets: &[Dataset], cfgs: &[Config], jobs: usize) -> Vec<Vec<f64>> {
    let mut out: Vec<Option<Vec<f64>>> = (0..sets.len()).map(|_| None).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::Mutex::new(&mut out);
    thread::scope(|sc| {
        for _ in 0..jobs {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= sets.len() {
                    break;
                }
                let r = row(&sets[i], cfgs);
                eprintln!("ablation: {} done", sets[i].name);
                done.lock().unwrap()[i] = Some(r);
            });
        }
    });
    out.into_iter().map(Option::unwrap).collect()
}

fn geomean(xs: impl Iterator<Item = f64>) -> f64 {
    let (s, n) = xs.fold((0.0, 0usize), |(s, n), x| (s + x.ln(), n + 1));
    (s / n as f64).exp()
}

fn table(sets: &[Dataset], cfgs: &[Config], rows: &[Vec<f64>]) {
    let mut head = vec!["configuration".to_string()];
    head.extend(sets.iter().map(|d| d.name.to_string()));
    println!("| {} |", head.join(" | "));
    println!("|---|{}", "---:|".repeat(sets.len()));
    let names = cfgs
        .iter()
        .map(|c| c.name)
        .chain(["rule-aware, fixed-width codes"]);
    for (j, name) in names.enumerate() {
        let cells: Vec<String> = rows.iter().map(|r| format!("{:.2}", r[j])).collect();
        println!("| {name} | {} |", cells.join(" | "));
    }
}

/// Mean bits per value and geometric mean of the ratio to the full analytic
/// codec (`+ lazy steps`, index 2), per group of datasets.
fn summary(groups: &[(&str, Vec<&Vec<f64>>)], cfgs: &[Config]) {
    let mut head = vec!["configuration".to_string()];
    for (g, r) in groups {
        head.push(format!("{g} mean ({})", r.len()));
        head.push(format!("{g} ÷ rule-aware"));
    }
    println!("| {} |", head.join(" | "));
    println!("|---|{}", "---:|".repeat(2 * groups.len()));
    let names = cfgs
        .iter()
        .map(|c| c.name)
        .chain(["rule-aware, fixed-width codes"]);
    for (j, name) in names.enumerate() {
        let mut cells = Vec::new();
        for (_, rs) in groups {
            let mean = rs.iter().map(|r| r[j]).sum::<f64>() / rs.len() as f64;
            let ratio = geomean(rs.iter().map(|r| r[j].max(1e-3) / r[2].max(1e-3)));
            cells.push(format!("{mean:.2}"));
            cells.push(format!("{ratio:.2}"));
        }
        println!("| {name} | {} |", cells.join(" | "));
    }
}

pub fn ablation(days: f64, real: Option<&std::path::Path>, jobs: usize) {
    let cfgs = configs();
    println!("\n## Ablation\n");
    println!(
        "Bits per value. Ladder: each row adds one component to the previous one, from the plain"
    );
    println!("Theorem 1 box budget with nearest-grid rounding to the full codec. Leave-one-out rows remove one");
    println!("component from the full analytic (`rule-aware`) or optimistic (`κ`, κ={RELAX_KAPPA}) codec. The last row");
    println!(
        "codes the full analytic plan with fixed-width residual codes instead of the range coder."
    );
    println!("Zero changed decisions in every cell (asserted).\n");

    let synth: Vec<Dataset> = datasets(days).into();
    let rs = rows(&synth, &cfgs, jobs);
    println!("Synthetic, {days} days:\n");
    table(&synth, &cfgs, &rs);

    let mut groups: Vec<(&str, Vec<&Vec<f64>>)> = vec![("synthetic", rs.iter().collect())];
    let (real_sets, real_rows);
    if let Some(dir) = real {
        let r = crate::real::load(dir);
        real_sets = crate::real::datasets(&r)
            .into_iter()
            .filter(|d| !d.name.ends_with("@prod"))
            .collect::<Vec<_>>();
        real_rows = rows(&real_sets, &cfgs, jobs);
        println!(
            "\n<details><summary>Real series ({}), q95 / q50 thresholds</summary>\n",
            dir.display()
        );
        table(&real_sets, &cfgs, &real_rows);
        println!("\n</details>");
        for v in ["q95", "q50"] {
            let g: Vec<&Vec<f64>> = real_sets
                .iter()
                .zip(&real_rows)
                .filter(|(d, _)| d.name.ends_with(&format!("@{v}")))
                .map(|(_, r)| r)
                .collect();
            groups.push((if v == "q95" { "real q95" } else { "real q50" }, g));
        }
    }
    let all: Vec<&Vec<f64>> = groups.iter().flat_map(|(_, r)| r.iter().copied()).collect();
    let gains: Vec<f64> = all
        .iter()
        .map(|r| r[2] - r[4])
        .filter(|&d| d > 0.0)
        .collect();
    println!(
        "\nScale search (`rule-aware + scale search` vs `rule-aware`) is smaller on {} of {} datasets, by at most {:.4} bits per value.",
        gains.len(),
        all.len(),
        gains.iter().copied().fold(0.0, f64::max)
    );
    println!("\nSummary: mean bits per value and geometric mean of the ratio to `rule-aware` (+ lazy steps):\n");
    summary(&groups, &cfgs);
}
