//! Seeded synthetic monitoring series with injected incidents.
//!
//! These are stand-ins until real cluster traces are wired in: each generator
//! mimics the value distribution of a common metric family and injects
//! incidents that make its alert rules fire a few times per day.

use std::f64::consts::TAU;

use alertsafe_rules::{Agg, Cmp, Rule, Series};

pub const SCRAPE_MS: i64 = 15_000;
pub const DAY_MS: i64 = 86_400_000;
const MIN: i64 = 60_000;

/// SplitMix64: tiny, seedable, dependency-free.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    pub fn normal(&mut self) -> f64 {
        let u = self.uniform().max(1e-300);
        (-2.0 * u.ln()).sqrt() * (TAU * self.uniform()).cos()
    }
}

pub struct Dataset {
    pub name: &'static str,
    pub series: Series,
    pub rules: Vec<Rule>,
}

/// Scrape timestamps with ±200ms jitter.
fn timestamps(rng: &mut Rng, days: f64) -> Vec<i64> {
    let n = (days * DAY_MS as f64 / SCRAPE_MS as f64) as i64;
    (0..n)
        .map(|i| i * SCRAPE_MS + (rng.range(-200.0, 200.0) as i64))
        .collect()
}

/// Sum of trapezoidal incident bumps, `per_day` on average (Poisson arrivals).
fn incidents(
    rng: &mut Rng,
    ts: &[i64],
    per_day: f64,
    dur_min: (f64, f64),
    amp: (f64, f64),
) -> Vec<f64> {
    let mut out = vec![0.0; ts.len()];
    let horizon = *ts.last().unwrap() as f64;
    let mut t = 0.0;
    loop {
        t += -rng.uniform().max(1e-12).ln() * DAY_MS as f64 / per_day;
        if t >= horizon {
            return out;
        }
        let dur = rng.range(dur_min.0, dur_min.1) * MIN as f64;
        let a = rng.range(amp.0, amp.1);
        for (o, &s) in out.iter_mut().zip(ts) {
            let u = (s as f64 - t) / dur;
            if (0.0..1.0).contains(&u) {
                *o += a * (u / 0.2).min((1.0 - u) / 0.2).min(1.0);
            }
        }
    }
}

fn diurnal(t: i64) -> f64 {
    (TAU * t as f64 / DAY_MS as f64).sin()
}

/// CPU utilisation ratio in [0, 1].
pub fn cpu(seed: u64, days: f64, threshold: f64, per_day: f64) -> Dataset {
    let mut rng = Rng::new(seed);
    let ts = timestamps(&mut rng, days);
    let bumps = incidents(&mut rng, &ts, per_day, (5.0, 40.0), (0.35, 0.6));
    let mut ar = 0.0;
    let vals = ts
        .iter()
        .zip(&bumps)
        .map(|(&t, b)| {
            ar = 0.95 * ar + 0.02 * rng.normal();
            (0.35 + 0.15 * diurnal(t) + ar + b).clamp(0.0, 1.0)
        })
        .collect();
    Dataset {
        name: "cpu",
        series: Series::new(ts, vals),
        rules: vec![
            Rule::new("HighCPU", Agg::AvgOverTime, 5 * MIN, Cmp::Gt, threshold).hold(10 * MIN),
            Rule::new("CPUSaturated", Agg::MaxOverTime, MIN, Cmp::Gt, 0.97).hold(2 * MIN),
        ],
    }
}

/// p99 request latency in seconds (log-normal around 120ms).
pub fn latency(seed: u64, days: f64) -> Dataset {
    let mut rng = Rng::new(seed);
    let ts = timestamps(&mut rng, days);
    let bumps = incidents(&mut rng, &ts, 2.0, (3.0, 30.0), (1.2, 2.5));
    let mut ar = 0.0;
    let vals = ts
        .iter()
        .zip(&bumps)
        .map(|(&t, b)| {
            ar = 0.9 * ar + 0.12 * rng.normal();
            (0.12f64.ln() + 0.2 * diurnal(t) + ar + b).exp()
        })
        .collect();
    Dataset {
        name: "latency",
        series: Series::new(ts, vals),
        rules: vec![
            Rule::new("LatencyHigh", Agg::AvgOverTime, 5 * MIN, Cmp::Gt, 0.5).hold(5 * MIN),
            Rule::new("LatencyCritical", Agg::Last, 0, Cmp::Gt, 1.0)
                .hold(2 * MIN)
                .keep_firing(5 * MIN),
        ],
    }
}

/// Resident memory in bytes with a slow leak and periodic restarts.
pub fn memory(seed: u64, days: f64) -> Dataset {
    let mut rng = Rng::new(seed);
    let ts = timestamps(&mut rng, days);
    let period = 1.7 * DAY_MS as f64;
    let vals = ts
        .iter()
        .map(|&t| {
            let phase = (t as f64).rem_euclid(period) / period;
            2.0e9 + 2.8e9 * phase.powf(1.5) + 1.5e8 * diurnal(t) + 2.0e7 * rng.normal()
        })
        .collect();
    Dataset {
        name: "memory",
        series: Series::new(ts, vals),
        rules: vec![
            Rule::new("MemoryHigh", Agg::Last, 0, Cmp::Gt, 4.5e9).hold(15 * MIN),
            Rule::new("MemoryGrowing", Agg::AvgOverTime, 30 * MIN, Cmp::Gt, 4.0e9).hold(30 * MIN),
        ],
    }
}

/// `http_requests_total` counter with diurnal traffic, bursts and restarts.
pub fn requests(seed: u64, days: f64) -> Dataset {
    let mut rng = Rng::new(seed);
    let ts = timestamps(&mut rng, days);
    let bursts = incidents(&mut rng, &ts, 1.5, (5.0, 30.0), (1.5, 3.0));
    let drops = incidents(&mut rng, &ts, 0.5, (15.0, 40.0), (0.9, 1.0));
    let restart_every = (2.5 * DAY_MS as f64 / SCRAPE_MS as f64) as usize;
    let mut c = 0.0f64;
    let mut prev_t = ts[0] - SCRAPE_MS;
    let vals = ts
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let rps = (200.0 + 120.0 * diurnal(t)) * (1.0 + bursts[i]) * (1.0 - drops[i]).max(0.02);
            let mean = rps * (t - prev_t) as f64 / 1000.0;
            prev_t = t;
            let inc = (mean + mean.sqrt() * rng.normal()).max(0.0).round();
            c = if i > 0 && i % restart_every == 0 {
                inc
            } else {
                c + inc
            };
            c
        })
        .collect();
    Dataset {
        name: "requests",
        series: Series::new(ts, vals),
        rules: vec![
            Rule::new("TrafficSpike", Agg::Rate, 5 * MIN, Cmp::Gt, 700.0).hold(5 * MIN),
            Rule::new("TrafficDrop", Agg::Rate, 5 * MIN, Cmp::Lt, 20.0).hold(10 * MIN),
        ],
    }
}
