//! Bias-invariant digram typing-speed model.
//!
//! Observed interval decomposition: T_ij = s_ij + b_i - b_j + noise.
//! Per-key "lateness" is absorbed by the bias terms b, leaving s_ij as the
//! intrinsic transition cost. Updates are O(1) online gradient steps.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const BASELINE_MS: f64 = 250.0;
/// Intervals longer than this are treated as pauses and discarded.
pub const PAUSE_THRESHOLD_MS: f64 = 2000.0;
/// Residuals are clamped to this magnitude before updates (robust loss).
const RESIDUAL_CLAMP_MS: f64 = 400.0;
/// Fixed learning rate floor for pair speeds.
const LR_FLOOR: f64 = 0.05;
/// Learning rate for bias nodes (kept small; biases are shared across pairs).
const LR_BIAS: f64 = 0.02;
/// Pseudo-count of baseline prior blended into estimates for display/scheduling.
const PRIOR_N: f64 = 3.0;
/// Prior accuracy blended into per-pair accuracy estimates.
const ACC_PRIOR: f64 = 0.97;
/// Pseudo-count for the accuracy prior.
const ACC_PRIOR_N: f64 = 10.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PairStat {
    /// Intrinsic transition cost estimate in ms.
    pub s: f64,
    /// Number of observations.
    pub n: u32,
    /// Correct attempts at this transition (serde default for migration).
    #[serde(default)]
    pub ok: u32,
    /// Errored attempts at this transition.
    #[serde(default)]
    pub err: u32,
}

impl Default for PairStat {
    fn default() -> Self {
        PairStat {
            s: BASELINE_MS,
            n: 0,
            ok: 0,
            err: 0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Model {
    /// Pair speeds keyed by two-char string, e.g. "th".
    pub pairs: HashMap<String, PairStat>,
    /// Per-key delay biases in ms.
    pub biases: HashMap<char, f64>,
    /// Running global mean interval (ms). Bias updates are measured against
    /// this rather than the per-pair speed: since each pair has its own free
    /// parameter, errors against the full model are always absorbable by s
    /// alone and biases would never be identified. Fitting biases to the
    /// antisymmetric per-key deviation from the global mean makes the
    /// decomposition identifiable (ANOVA-style).
    pub global_mu: f64,
    /// Total observations across all pairs.
    pub total_obs: u64,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            pairs: HashMap::new(),
            biases: HashMap::new(),
            global_mu: BASELINE_MS,
            total_obs: 0,
        }
    }
}

fn pair_key(a: char, b: char) -> String {
    let mut k = String::with_capacity(2);
    k.push(a);
    k.push(b);
    k
}

impl Model {
    /// Observe key `a` followed by key `b` after `dt_ms`. Returns false if
    /// the observation was rejected as an outlier/pause.
    pub fn observe(&mut self, a: char, b: char, dt_ms: f64) -> bool {
        if !(0.0..PAUSE_THRESHOLD_MS).contains(&dt_ms) {
            return false;
        }
        let b_a = *self.biases.get(&a).unwrap_or(&0.0);
        let b_b = *self.biases.get(&b).unwrap_or(&0.0);

        // Track the global mean interval.
        self.global_mu += 0.02 * (dt_ms - self.global_mu);

        // Biases fit the antisymmetric per-key deviation from the global mean.
        let err_bias =
            (dt_ms - (self.global_mu + b_a - b_b)).clamp(-RESIDUAL_CLAMP_MS, RESIDUAL_CLAMP_MS);
        *self.biases.entry(a).or_insert(0.0) += LR_BIAS * err_bias;
        *self.biases.entry(b).or_insert(0.0) -= LR_BIAS * err_bias;

        // Pair speed fits the bias-corrected interval: s -> E[dt] - b_a + b_b.
        let stat = self.pairs.entry(pair_key(a, b)).or_default();
        let err_pair = (dt_ms - (stat.s + b_a - b_b)).clamp(-RESIDUAL_CLAMP_MS, RESIDUAL_CLAMP_MS);
        // Count-based learning rate with a floor, so early observations move
        // the estimate quickly and later ones refine it.
        let lr = (1.0 / (stat.n as f64 + 1.0)).max(LR_FLOOR);
        stat.s += lr * err_pair;
        stat.n = stat.n.saturating_add(1);

        self.total_obs += 1;
        true
    }

    /// Confidence-blended speed estimate for a pair (prior pulls toward baseline).
    pub fn pair_speed(&self, a: char, b: char) -> f64 {
        match self.pairs.get(&pair_key(a, b)) {
            Some(st) => (st.n as f64 * st.s + PRIOR_N * BASELINE_MS) / (st.n as f64 + PRIOR_N),
            None => BASELINE_MS,
        }
    }

    pub fn pair_count(&self, a: char, b: char) -> u32 {
        self.pairs.get(&pair_key(a, b)).map_or(0, |st| st.n)
    }

    /// True if this pair has any speed observations or accuracy attempts.
    pub fn has_pair(&self, a: char, b: char) -> bool {
        self.pairs.contains_key(&pair_key(a, b))
    }

    /// Record whether an attempt at the transition a->b was typed correctly.
    /// Errors of any kind (extra key, skip, substitution) count against the
    /// transition into the expected character.
    pub fn record_attempt(&mut self, a: char, b: char, correct: bool) {
        let stat = self.pairs.entry(pair_key(a, b)).or_default();
        if correct {
            stat.ok = stat.ok.saturating_add(1);
        } else {
            stat.err = stat.err.saturating_add(1);
        }
    }

    /// Raw (ok, err) attempt counts for a transition.
    pub fn pair_attempts(&self, a: char, b: char) -> (u32, u32) {
        self.pairs
            .get(&pair_key(a, b))
            .map_or((0, 0), |st| (st.ok, st.err))
    }

    /// Prior-blended accuracy estimate for a transition in [0, 1].
    pub fn pair_accuracy(&self, a: char, b: char) -> f64 {
        let (ok, err) = self
            .pairs
            .get(&pair_key(a, b))
            .map_or((0.0, 0.0), |st| (st.ok as f64, st.err as f64));
        (ok + ACC_PRIOR * ACC_PRIOR_N) / (ok + err + ACC_PRIOR_N)
    }

    /// Population-frequency-weighted accuracy: invariant to how hard the
    /// currently presented material is (same normalization as WPM).
    pub fn normalized_accuracy(&self, freqs: &HashMap<(char, char), f64>) -> f64 {
        let mut num = 0.0;
        let mut den = 0.0;
        for (&(a, b), &f) in freqs {
            num += f * self.pair_accuracy(a, b);
            den += f;
        }
        if den > 0.0 { num / den } else { ACC_PRIOR }
    }

    /// Remove gauge freedom: biases are only identifiable up to a global
    /// constant, so re-center them to mean zero. Call periodically.
    pub fn recenter_biases(&mut self) {
        if self.biases.is_empty() {
            return;
        }
        let mean = self.biases.values().sum::<f64>() / self.biases.len() as f64;
        for v in self.biases.values_mut() {
            *v -= mean;
        }
    }

    /// Population-frequency-weighted mean interval (ms per keystroke).
    /// `freqs` maps digrams to population frequencies. This makes displayed
    /// WPM invariant to the difficulty of currently presented material.
    pub fn normalized_interval_ms(&self, freqs: &HashMap<(char, char), f64>) -> f64 {
        let mut num = 0.0;
        let mut den = 0.0;
        for (&(a, b), &f) in freqs {
            num += f * self.pair_speed(a, b);
            den += f;
        }
        if den > 0.0 { num / den } else { BASELINE_MS }
    }

    /// Normalized WPM (standard 5 chars per word).
    pub fn normalized_wpm(&self, freqs: &HashMap<(char, char), f64>) -> f64 {
        60_000.0 / (self.normalized_interval_ms(freqs) * 5.0)
    }

    /// Population-frequency-weighted speed of a single key: average intrinsic
    /// cost of transitions involving this key. Used for the heatmap.
    pub fn key_speed(&self, c: char, freqs: &HashMap<(char, char), f64>) -> f64 {
        let mut num = 0.0;
        let mut den = 0.0;
        for (&(a, b), &f) in freqs {
            if a == c || b == c {
                num += f * self.pair_speed(a, b);
                den += f;
            }
        }
        if den > 0.0 { num / den } else { BASELINE_MS }
    }

    /// Population-frequency-weighted accuracy of a single key across
    /// transitions involving it. Used for the per-key accuracy display.
    pub fn key_accuracy(&self, c: char, freqs: &HashMap<(char, char), f64>) -> f64 {
        let mut num = 0.0;
        let mut den = 0.0;
        for (&(a, b), &f) in freqs {
            if a == c || b == c {
                num += f * self.pair_accuracy(a, b);
                den += f;
            }
        }
        if den > 0.0 { num / den } else { ACC_PRIOR }
    }

    pub fn key_bias(&self, c: char) -> f64 {
        *self.biases.get(&c).unwrap_or(&0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn train(model: &mut Model, samples: &[(char, char, f64)], reps: usize) {
        for _ in 0..reps {
            for &(a, b, dt) in samples {
                model.observe(a, b, dt);
            }
            model.recenter_biases();
        }
    }

    #[test]
    fn converges_to_observed_interval() {
        let mut m = Model::default();
        train(&mut m, &[('t', 'h', 120.0), ('h', 't', 120.0)], 200);
        assert!((m.pair_speed('t', 'h') - 120.0).abs() < 10.0);
    }

    #[test]
    fn bias_invariance() {
        // Two datasets identical except key 'x' fires 50ms late everywhere.
        // Intrinsic pair speeds should come out (nearly) the same.
        let base: Vec<(char, char, f64)> =
            vec![('a', 'x', 150.0), ('x', 'b', 150.0), ('b', 'a', 150.0)];
        let shifted: Vec<(char, char, f64)> = vec![
            ('a', 'x', 200.0), // interval ending at x grows by 50
            ('x', 'b', 100.0), // interval starting at x shrinks by 50
            ('b', 'a', 150.0),
        ];
        let mut m1 = Model::default();
        let mut m2 = Model::default();
        train(&mut m1, &base, 500);
        train(&mut m2, &shifted, 500);
        for &(a, b) in &[('a', 'x'), ('x', 'b'), ('b', 'a')] {
            let d = (m1.pair_speed(a, b) - m2.pair_speed(a, b)).abs();
            assert!(d < 15.0, "pair {}{} differs by {:.1}ms", a, b, d);
        }
        // The lateness shows up in the bias term instead.
        assert!(m2.key_bias('x') < m1.key_bias('x') - 20.0);
    }

    #[test]
    fn rejects_pauses() {
        let mut m = Model::default();
        assert!(!m.observe('a', 'b', 5000.0));
        assert_eq!(m.pair_count('a', 'b'), 0);
    }
}
