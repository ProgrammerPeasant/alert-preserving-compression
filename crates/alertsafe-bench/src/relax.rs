//! H7: optimistic budget with a posteriori repair.
//!
//! The analytic budget of Theorem 1 holds for every reconstruction inside a
//! box, i.e. for the worst case of every sample error at once. Real errors
//! mostly cancel in window aggregates, so a budget `κ` times larger usually
//! changes only a few evaluations. Verify-and-repair finds those and shrinks
//! the budget on their supports only. The guarantee is end to end (checked
//! against the reference evaluator), as for the analytic budget.

use alertsafe_codec::{alert_diff, budget, decode, encode_chunked, flipped};

use crate::gen::Dataset;
use crate::{datasets, eval_grid};

/// Slack used where a single relaxed configuration is reported.
pub const RELAX_KAPPA: f64 = 1024.0;

/// Optimistic budget `kappa * eps_i`, then a posteriori repair
/// that halves the budget on the support of every flipped evaluation, never
/// below the analytic budget (where the condition is guaranteed); zeroing as
/// the last resort for floating-point ties. Returns (bits/value, rounds).
pub fn relaxed(ds: &Dataset, kappa: f64) -> (f64, usize) {
    let s = &ds.series;
    let g = eval_grid(s);
    let plan = budget::plan(s, &ds.rules, &g);
    let base = plan.budget.clone();
    let mut b: Vec<f64> = base.iter().map(|&e| e * kappa).collect();
    let mut rounds = 0;
    loop {
        let enc = encode_chunked(&s.vals, &b, plan.mode, usize::MAX);
        let recon = s.with_values(enc.iter().flat_map(|e| decode(&e.bytes)).collect());
        let flips = flipped(s, &recon, &ds.rules, &g);
        if flips.is_empty() {
            assert_eq!(alert_diff(s, &recon, &ds.rules, &g).state_mismatches, 0);
            let bits: usize = enc.iter().map(|e| e.bits).sum();
            return (bits as f64 / s.len() as f64, rounds);
        }
        rounds += 1;
        // Decide on the budgets this round was encoded with: supports overlap.
        let prev = b.clone();
        for (rule, t) in flips {
            let sup = rule.support(&s.ts, t);
            let relaxed = sup.clone().any(|i| prev[i] > base[i]);
            for i in sup {
                b[i] = if relaxed {
                    b[i].min((prev[i] * 0.5).max(base[i]))
                } else {
                    0.0
                };
            }
        }
    }
}

pub fn h7(days: f64) {
    println!("\n## H7: optimistic budget with a posteriori repair ({days} days)\n");
    println!("Bits per value with the analytic budget scaled by κ, then repaired: on every flipped evaluation the");
    println!("budget on its support is halved, never below the analytic one, and zeroed if already there. In");
    println!("parentheses: repair rounds (one encode + verify each). Zero changed decisions everywhere (asserted).\n");
    println!("| dataset | κ=1 (analytic) | κ=2 | κ=4 | κ=16 | κ=64 | κ=1024 |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for ds in &datasets(days) {
        let cells: Vec<String> = [1.0, 2.0, 4.0, 16.0, 64.0, 1024.0]
            .iter()
            .map(|&k| {
                let (b, r) = relaxed(ds, k);
                format!("{b:.2} ({r})")
            })
            .collect();
        println!("| {} | {} |", ds.name, cells.join(" | "));
    }
}
