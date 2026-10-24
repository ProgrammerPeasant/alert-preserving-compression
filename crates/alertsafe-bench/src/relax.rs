//! H7: optimistic budget with a posteriori repair ([`Options::slack`]).
//!
//! The analytic budget of Theorem 1 holds for every reconstruction inside a
//! box, i.e. for the worst case of every sample error at once. Real errors
//! mostly cancel in window aggregates, so a budget `κ` times larger usually
//! changes only a few evaluations. Verify-and-repair finds those and shrinks
//! the budget on their supports only. The guarantee is end to end (checked
//! against the reference evaluator), as for the analytic budget.

use alertsafe_codec::{alert_diff, compress_with, Options};

use crate::gen::Dataset;
use crate::{datasets, eval_grid, PROM_CHUNK};

/// Slack used where a single relaxed configuration is reported.
pub const RELAX_KAPPA: f64 = 1024.0;

/// Bits per value and repair rounds with the analytic budget scaled by
/// `kappa`, coded in streams of `chunk` samples.
pub fn relaxed(ds: &Dataset, kappa: f64, chunk: usize) -> (f64, usize) {
    let s = &ds.series;
    let g = eval_grid(s);
    let opts = Options {
        chunk,
        slack: kappa,
        ..Default::default()
    };
    let c = compress_with(s, &ds.rules, &g, opts);
    let recon = s.with_values(c.decode());
    assert_eq!(alert_diff(s, &recon, &ds.rules, &g).state_mismatches, 0);
    (c.bits_per_value(), c.rounds)
}

pub fn h7(days: f64) {
    println!("\n## H7: optimistic budget with a posteriori repair ({days} days)\n");
    println!("Bits per value with the analytic budget scaled by κ, then repaired: on every flipped evaluation the");
    println!("budget on its support is halved, never below the analytic one, and zeroed if already there. In");
    println!("parentheses: repair rounds (one encode + verify each). Zero changed decisions everywhere (asserted).\n");
    let kappas = [1.0, 2.0, 4.0, 16.0, 64.0, 1024.0];
    for (chunk, label) in [(usize::MAX, "whole series"), (PROM_CHUNK, "chunks of 120")] {
        println!("\n{label}:\n");
        println!("| dataset | κ=1 (analytic) | κ=2 | κ=4 | κ=16 | κ=64 | κ=1024 |");
        println!("|---|---:|---:|---:|---:|---:|---:|");
        for ds in &datasets(days) {
            let cells: Vec<String> = kappas
                .iter()
                .map(|&k| {
                    let (b, r) = relaxed(ds, k, chunk);
                    format!("{b:.2} ({r})")
                })
                .collect();
            println!("| {} | {} |", ds.name, cells.join(" | "));
        }
    }
}
