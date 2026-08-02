//! Per-criterion drift detection over the tracked trend history.
//!
//! A scenario's pass rate can move for reasons that have nothing to do with the run reporting it — a
//! prompt edit, a new namespace, a changed default — and at the sample sizes a suite run affords, a
//! single run cannot tell a real move from an unlucky draw. Judging each run against a *bar* answers
//! "did it clear the line", never "is this the same scenario it was last week", so a base rate can
//! slide a long way while every run still reports green.
//!
//! This reads the accumulated per-criterion tallies in `eval/history.jsonl` and asks the second
//! question. Two choices make it answer something a per-run check cannot:
//!
//! - **It pools.** A criterion's recent runs are compared as one sample against the pooled prior, so a
//!   drop that hides inside one run's noise surfaces once a second run agrees with it. A rate that
//!   fell from ~0.9 to a true 0.65 shows first as an unremarkable 5/5, then as 1/5; pooled, 6/10
//!   against that prior is improbable enough to report, where neither run alone is anything at all.
//! - **It brackets.** Every history line records the commit it ran at, so a flagged criterion names
//!   the range between the last run consistent with the old rate and the first that is not. That turns
//!   "something regressed sometime" into a handful of commits to read.
//!
//! Drift is **advisory and never gating**. A deliberate improvement moves a rate exactly as loudly as
//! a regression, and a rise is worth reading rather than suppressing: a criterion that jumps to a
//! perfect rate has often stopped being able to fail — a fixture that no longer exercises the path it
//! names reports as success.

use std::collections::{BTreeMap, BTreeSet};

use crate::history::HistoryLine;

/// A criterion's identity across runs: the scenario it belongs to, and its own name. Criteria are only
/// comparable within a scenario — two scenarios can and do word a criterion identically.
pub(crate) type CriterionKey = (String, String);

/// One run's tally for a criterion, carrying the commit that run ran at so a drift can be bracketed to
/// the range it appeared in.
struct Measured {
    tally: (u32, u32),
    sha: String,
}

/// One criterion whose recent rate does not look like a sample from its own history.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drift {
    pub(crate) scenario: String,
    pub(crate) criterion: String,
    /// Pooled `(passed, total)` over the runs before the recent window.
    pub(crate) prior: (u32, u32),
    /// Pooled `(passed, total)` over the recent window.
    pub(crate) recent: (u32, u32),
    /// The probability of a result at least this extreme under the prior rate — the smaller, the less
    /// the move looks like sampling.
    pub(crate) p_value: f64,
    /// The commit of the last run before the recent window, and of the first run within it: the range
    /// the change is bracketed to. Either may be empty when git could not resolve one.
    pub(crate) since: String,
    pub(crate) until: String,
}

impl Drift {
    /// Whether the rate moved down (a possible regression) rather than up.
    pub(crate) fn fell(&self) -> bool {
        rate(self.recent) < rate(self.prior)
    }
}

/// The prior rate to test against, with one notional pass and one notional failure added (a Laplace
/// smoothing). Most criteria sit at a perfect tally for long stretches, and taking that literally makes
/// the rate exactly 1.0 — under which a single failure has probability zero, so the very first one ever
/// seen is flagged as infinitely improbable no matter how the window is pooled. A run of successes is
/// evidence that failure is *rare*, not that it cannot happen, and the smoothing says so: 48/48 reads
/// as 0.98, where one slip is unremarkable and a pooled 6/10 is still decisive.
fn smoothed((passed, total): (u32, u32)) -> f64 {
    f64::from(passed + 1) / f64::from(total + 2)
}

/// The rate a `(passed, total)` tally represents; zero for an empty tally, which no caller reaches
/// because a criterion with no runs is never compared.
pub(crate) fn rate((passed, total): (u32, u32)) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(passed) / f64::from(total)
    }
}

/// How many trailing runs form the recent window. Two is the smallest that can pool — one run alone is
/// what this exists to improve on — and keeping it small keeps the bracket tight, since the flagged
/// commit range spans the window's own runs.
const RECENT_RUNS: usize = 2;

/// A criterion is flagged when a result this extreme has below this probability under its prior rate.
///
/// Set where a genuine, costly drift actually lands rather than at a comfortable-sounding round number.
/// The org-namespace case this was built for (see the tests) pools to 6/10 against a ~0.9 prior, which
/// is p ≈ 0.018 — real evidence, but not the decisive number it looks like from the rates alone. A
/// tighter 0.01 threshold reads as more rigorous and silently misses it, which is the failure mode that
/// matters here: the flag costs a reader one glance at a commit range, while a miss costs a week.
/// Because this is advisory and never gating, the asymmetry favours sensitivity.
const P_THRESHOLD: f64 = 0.05;

/// The smallest rate change worth reporting. A large history can make a trivial move statistically
/// significant — 0.98 against 0.94 over hundreds of runs — which is true and useless. A criterion has
/// to have moved by this much *and* be improbable to be flagged.
const MIN_SHIFT: f64 = 0.15;

/// The minimum pooled prior runs before a criterion is judged at all. Below this the prior rate is
/// itself a guess, and comparing against it manufactures signal from noise.
const MIN_PRIOR: u32 = 8;

/// The minimum number of separate history lines the prior must span. A single run can satisfy
/// [`MIN_PRIOR`] on its own at a large enough N, but one run is one draw of whatever conditions held
/// that day; a prior worth testing against has survived a few of them.
const MIN_PRIOR_LINES: usize = 3;

/// Detect drifted criteria across `lines`, oldest first.
///
/// Lines are deduplicated by run name, last occurrence winning: re-running a name (a resume, or a
/// `--retry-infra-failed` heal) appends a fresh row for the same run, and counting both would double a
/// run's weight and let the pre-heal tallies — the ones the heal exists to supersede — pull the prior
/// rate around.
///
/// Only the `(scenario, criterion)` pairs in `measured` are considered — in practice, those the run
/// being reported actually measured. Without that anchor a criterion's "recent" window is simply the
/// last runs that happened to contain it, so one retired months ago reports its final runs as though
/// they were current, and a criterion that has since been renamed or dropped drifts forever.
pub(crate) fn detect(lines: &[HistoryLine], measured: &BTreeSet<CriterionKey>) -> Vec<Drift> {
    let mut series: BTreeMap<CriterionKey, Vec<Measured>> = BTreeMap::new();
    for line in dedupe_by_name(lines) {
        for scenario in &line.scenarios {
            for stat in &scenario.criteria {
                series
                    .entry((scenario.name.clone(), stat.criterion.clone()))
                    .or_default()
                    .push(Measured {
                        tally: (stat.passed, stat.total),
                        sha: line.git_sha.clone(),
                    });
            }
        }
    }

    let mut drifts = Vec::new();
    for ((scenario, criterion), runs) in series {
        if !measured.contains(&(scenario.clone(), criterion.clone())) {
            continue;
        }
        if runs.len() <= RECENT_RUNS {
            continue;
        }
        let split = runs.len() - RECENT_RUNS;
        let (before, after) = runs.split_at(split);
        if before.len() < MIN_PRIOR_LINES {
            continue;
        }
        let prior = pool(before.iter().map(|run| run.tally));
        let recent = pool(after.iter().map(|run| run.tally));
        if prior.1 < MIN_PRIOR || recent.1 == 0 {
            continue;
        }
        if (rate(recent) - rate(prior)).abs() < MIN_SHIFT {
            continue;
        }
        let p_value = tail_probability(recent, smoothed(prior));
        if p_value >= P_THRESHOLD {
            continue;
        }
        drifts.push(Drift {
            scenario,
            criterion,
            prior,
            recent,
            p_value,
            since: before.last().map(|run| run.sha.clone()).unwrap_or_default(),
            until: after.first().map(|run| run.sha.clone()).unwrap_or_default(),
        });
    }
    // Most improbable first: the strongest signal is what a reader should spend attention on.
    drifts.sort_by(|a, b| a.p_value.total_cmp(&b.p_value));
    drifts
}

/// The history lines with each run name reduced to its last occurrence, order otherwise preserved.
fn dedupe_by_name(lines: &[HistoryLine]) -> Vec<&HistoryLine> {
    let mut last: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, line) in lines.iter().enumerate() {
        last.insert(line.name.as_str(), i);
    }
    lines
        .iter()
        .enumerate()
        .filter(|(i, line)| last.get(line.name.as_str()) == Some(i))
        .map(|(_, line)| line)
        .collect()
}

/// Sum a series of `(passed, total)` tallies into one.
fn pool(tallies: impl Iterator<Item = (u32, u32)>) -> (u32, u32) {
    tallies.fold((0, 0), |(passed, total), (p, t)| (passed + p, total + t))
}

/// The probability of a result at least as extreme as `observed` under `expected`, in whichever
/// direction the observation moved — the lower tail for a fall, the upper for a rise. One-tailed by
/// construction rather than by choice: the direction is read off the data before the tail is taken, so
/// this is a screening statistic for ranking candidates, not a hypothesis test to quote.
fn tail_probability((passed, total): (u32, u32), expected: f64) -> f64 {
    if total == 0 {
        return 1.0;
    }
    let observed = f64::from(passed) / f64::from(total);
    let range = if observed < expected {
        0..=passed
    } else {
        passed..=total
    };
    range.map(|k| binomial_pmf(k, total, expected)).sum()
}

/// `P(X = k)` for `X ~ Binomial(n, p)`, summed in log space so a large `n` cannot overflow the
/// factorials the direct form would need.
fn binomial_pmf(k: u32, n: u32, p: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) {
        return 0.0;
    }
    // The degenerate rates have no logarithm; a prior of exactly 0 or 1 puts all mass on one outcome.
    if p == 0.0 {
        return if k == 0 { 1.0 } else { 0.0 };
    }
    if p == 1.0 {
        return if k == n { 1.0 } else { 0.0 };
    }
    let log_choose = log_factorial(n) - log_factorial(k) - log_factorial(n - k);
    (log_choose + f64::from(k) * p.ln() + f64::from(n - k) * (1.0 - p).ln()).exp()
}

/// `ln(n!)` by summation — exact enough at the sample sizes a suite produces, and free of the
/// approximation error a Stirling form would introduce at small `n`, which is where every comparison
/// here actually sits.
fn log_factorial(n: u32) -> f64 {
    (1..=n).map(|i| f64::from(i).ln()).sum()
}

#[cfg(test)]
mod tests;
