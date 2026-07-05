//! Word corpus, population digram frequencies, and adaptive lesson generation.

use crate::model::Model;
use rand::Rng;
use rand::seq::IndexedRandom;
use std::collections::HashMap;

/// Frequency-ranked English word list (google-10000-english).
const RAW_WORDS: &str = include_str!("../assets/words.txt");

/// Probability of picking a uniformly random word (exploration floor) instead
/// of a targeted one, keeping coverage across all pairs.
const EXPLORATION_P: f64 = 0.2;
/// Probability of picking a word containing one of the current focus pairs
/// (the learner's highest-payoff struggle digrams).
const FOCUS_P: f64 = 0.5;
/// Number of struggle digrams targeted at a time.
pub const FOCUS_K: usize = 5;
/// Uncertainty boost: pairs with few observations get their sampling weight
/// inflated by up to this factor.
const UNCERTAINTY_BOOST: f64 = 1.5;
/// Approximate time cost of making an error (notice + correct), used to fold
/// accuracy into a pair's effective practice payoff.
const ERROR_PENALTY_MS: f64 = 750.0;
/// Pseudo-count blending pair-level accuracy toward its key-level prior.
const ACC_HIER_N: f64 = 5.0;

pub struct Corpus {
    pub words: Vec<&'static str>,
    /// Zipf-weighted population digram frequencies (letters only).
    pub digram_freqs: HashMap<(char, char), f64>,
    /// Inverted index: digram -> indices of words containing it.
    index: HashMap<(char, char), Vec<u32>>,
}

impl Corpus {
    pub fn load() -> Self {
        let words: Vec<&'static str> = RAW_WORDS
            .lines()
            .map(str::trim)
            .filter(|w| w.len() >= 2 && w.chars().all(|c| c.is_ascii_lowercase()))
            .collect();

        // The list is frequency-ranked; approximate word frequency by Zipf's
        // law (1/rank) to build a population digram distribution, and build
        // an inverted index for targeted lesson generation.
        let mut digram_freqs: HashMap<(char, char), f64> = HashMap::new();
        let mut index: HashMap<(char, char), Vec<u32>> = HashMap::new();
        for (rank, word) in words.iter().enumerate() {
            let w = 1.0 / (rank as f64 + 1.0);
            let chars: Vec<char> = word.chars().collect();
            for pair in chars.windows(2) {
                let key = (pair[0], pair[1]);
                *digram_freqs.entry(key).or_insert(0.0) += w;
                let entry = index.entry(key).or_default();
                if entry.last() != Some(&(rank as u32)) {
                    entry.push(rank as u32);
                }
            }
        }
        let total: f64 = digram_freqs.values().sum();
        for v in digram_freqs.values_mut() {
            *v /= total;
        }

        Corpus {
            words,
            digram_freqs,
            index,
        }
    }

    /// Effective time cost of practicing errors into account: an error costs
    /// roughly this long to notice and correct, so a pair's effective cost is
    /// speed + error_rate x penalty.
    fn effective_cost(&self, model: &Model, a: char, b: char, key_acc: &[f64; 26]) -> f64 {
        // Hierarchical accuracy: the pair's prior is the WORSE of its two
        // keys' aggregate accuracies, so sparsely-observed pairs inherit a
        // known problem key (e.g. a struggling 'x') instead of being pulled
        // toward a global optimistic prior.
        let idx = |c: char| (c as u8 - b'a') as usize;
        let prior = key_acc[idx(a)].min(key_acc[idx(b)]);
        let (ok, err) = model.pair_attempts(a, b);
        let acc = (ok as f64 + prior * ACC_HIER_N) / ((ok + err) as f64 + ACC_HIER_N);
        model.pair_speed(a, b) + (1.0 - acc) * ERROR_PENALTY_MS
    }

    fn key_accuracies(&self, model: &Model) -> [f64; 26] {
        let mut acc = [1.0; 26];
        for (i, c) in ('a'..='z').enumerate() {
            acc[i] = model.key_accuracy(c, &self.digram_freqs);
        }
        acc
    }

    /// Rank digrams by expected payoff of practicing them: population
    /// frequency (softened by sqrt so rare pairs still surface) x effective
    /// cost deficit (speed AND error rate) vs the learner's overall mean x
    /// uncertainty boost. Returns the top `k` with their scores; empty when
    /// nothing is measurably weak yet.
    pub fn focus_pairs(&self, model: &Model, k: usize) -> Vec<((char, char), f64)> {
        let mu = model.normalized_interval_ms(&self.digram_freqs);
        let key_acc = self.key_accuracies(model);
        let mut scored: Vec<((char, char), f64)> = self
            .digram_freqs
            .iter()
            .filter_map(|(&pair, &f)| {
                if !pair.0.is_ascii_lowercase() || !pair.1.is_ascii_lowercase() {
                    return None;
                }
                let deficit = self.effective_cost(model, pair.0, pair.1, &key_acc) - mu;
                if deficit <= 0.0 {
                    return None;
                }
                let n = model.pair_count(pair.0, pair.1) as f64;
                let boost = 1.0 + (UNCERTAINTY_BOOST - 1.0) / (1.0 + n / 5.0);
                Some((pair, f.sqrt() * deficit * boost))
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scored.truncate(k);
        scored
    }

    /// Difficulty score of a word: mean sampling weight of its digrams, where
    /// weight grows with effective pair cost (speed and error rate) and with
    /// uncertainty.
    fn word_score(&self, word: &str, model: &Model, key_acc: &[f64; 26]) -> f64 {
        let chars: Vec<char> = word.chars().collect();
        let mut sum = 0.0;
        let mut n = 0.0;
        for pair in chars.windows(2) {
            let cost = self.effective_cost(model, pair[0], pair[1], key_acc);
            let count = model.pair_count(pair[0], pair[1]) as f64;
            let boost = 1.0 + (UNCERTAINTY_BOOST - 1.0) / (1.0 + count / 5.0);
            sum += cost * boost;
            n += 1.0;
        }
        if n > 0.0 { sum / n } else { 0.0 }
    }

    /// Generate a lesson of `n_words` from a three-way mix:
    /// - focus draws (FOCUS_P): words containing one of the learner's current
    ///   focus pairs, sampled proportionally to each pair's payoff score;
    /// - difficulty draws: softmax-weighted by overall word difficulty;
    /// - exploration draws (EXPLORATION_P): uniform, so nothing drops out of
    ///   rotation.
    pub fn generate_lesson<R: Rng>(&self, model: &Model, n_words: usize, rng: &mut R) -> String {
        let focus = self.focus_pairs(model, FOCUS_K);
        let focus_total: f64 = focus.iter().map(|(_, s)| s).sum();

        // Score a random candidate pool rather than all 10k words per lesson.
        let key_acc = self.key_accuracies(model);
        let pool: Vec<&&str> = self
            .words
            .choose_multiple(rng, 400.min(self.words.len()))
            .collect();
        let scores: Vec<f64> = pool
            .iter()
            .map(|w| self.word_score(w, model, &key_acc))
            .collect();
        let mean = scores.iter().sum::<f64>() / scores.len() as f64;

        // Softmax-ish weights relative to the mean difficulty.
        let weights: Vec<f64> = scores
            .iter()
            .map(|s| ((s - mean) / 40.0).exp().clamp(0.05, 20.0))
            .collect();
        let total_w: f64 = weights.iter().sum();

        let mut out: Vec<&str> = Vec::with_capacity(n_words);
        while out.len() < n_words {
            let roll = rng.random::<f64>();
            if roll < EXPLORATION_P {
                out.push(self.words.choose(rng).unwrap());
                continue;
            }
            // Focus draw: pick a struggle digram by payoff, then a word
            // containing it (Zipf-weighted toward common words).
            if roll < EXPLORATION_P + FOCUS_P && focus_total > 0.0 {
                let mut target = rng.random::<f64>() * focus_total;
                let pair = focus
                    .iter()
                    .find(|(_, s)| {
                        target -= s;
                        target <= 0.0
                    })
                    .map(|(p, _)| *p)
                    .unwrap_or(focus[0].0);
                if let Some(word) = self.sample_word_with(pair, rng) {
                    out.push(word);
                    continue;
                }
            }
            // Difficulty-weighted draw from the pool.
            let mut target = rng.random::<f64>() * total_w;
            for (i, w) in weights.iter().enumerate() {
                target -= w;
                if target <= 0.0 {
                    out.push(pool[i]);
                    break;
                }
            }
        }
        out.join(" ")
    }

    /// Sample a word containing `pair`, weighted toward common words (the
    /// index is rank-ordered; take a uniformly random prefix position squared
    /// to skew toward the front).
    fn sample_word_with<R: Rng>(&self, pair: (char, char), rng: &mut R) -> Option<&'static str> {
        let ids = self.index.get(&pair)?;
        let u = rng.random::<f64>();
        let i = ((u * u) * ids.len() as f64) as usize;
        Some(self.words[ids[i.min(ids.len() - 1)] as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_loads_and_freqs_normalized() {
        let c = Corpus::load();
        assert!(c.words.len() > 5000);
        let total: f64 = c.digram_freqs.values().sum();
        assert!((total - 1.0).abs() < 1e-9);
    }

    #[test]
    fn adaptive_lessons_oversample_slow_digrams() {
        let c = Corpus::load();
        let mut model = Model::default();
        // Simulate: transitions involving 'q' are very slow, all else fast.
        for (a, b) in c.digram_freqs.keys() {
            let dt = if *a == 'q' || *b == 'q' { 600.0 } else { 100.0 };
            for _ in 0..20 {
                model.observe(*a, *b, dt);
            }
        }
        let mut rng = rand::rng();
        let mut q_adaptive = 0;
        let mut q_uniform = 0;
        for _ in 0..50 {
            q_adaptive += c.generate_lesson(&model, 10, &mut rng).matches('q').count();
            let uniform: Vec<&&str> = c.words.choose_multiple(&mut rng, 10).collect();
            q_uniform += uniform.iter().filter(|w| w.contains('q')).count();
        }
        assert!(
            q_adaptive > q_uniform,
            "adaptive lessons should contain more 'q' words ({} vs {})",
            q_adaptive,
            q_uniform
        );
        // Normalized wpm should reflect population mix, dominated by fast pairs.
        let wpm = model.normalized_wpm(&c.digram_freqs);
        assert!(wpm > 60.0, "normalized wpm was {wpm}");
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    #[test]
    fn focus_targets_slow_common_digram() {
        let c = Corpus::load();
        let mut model = Model::default();
        // 'th' is slow, everything else fast.
        for (a, b) in c.digram_freqs.keys() {
            let dt = if (*a, *b) == ('t', 'h') { 500.0 } else { 100.0 };
            for _ in 0..20 {
                model.observe(*a, *b, dt);
            }
        }
        let focus = c.focus_pairs(&model, FOCUS_K);
        assert_eq!(focus[0].0, ('t', 'h'), "focus was {:?}", focus);

        // Lessons should contain far more 'th' words than uniform sampling.
        let mut rng = rand::rng();
        let mut th_lesson = 0;
        let mut th_uniform = 0;
        for _ in 0..50 {
            th_lesson += c
                .generate_lesson(&model, 10, &mut rng)
                .matches("th")
                .count();
            let uniform: Vec<&&str> = c.words.choose_multiple(&mut rng, 10).collect();
            th_uniform += uniform
                .iter()
                .map(|w| w.matches("th").count())
                .sum::<usize>();
        }
        assert!(
            th_lesson > th_uniform * 2,
            "lesson 'th' count {} vs uniform {}",
            th_lesson,
            th_uniform
        );
    }
}

#[cfg(test)]
mod accuracy_focus_tests {
    use super::*;

    #[test]
    fn error_prone_key_gets_focus_despite_normal_speed() {
        let c = Corpus::load();
        let mut model = Model::default();
        // Uniform speed everywhere; transitions involving 'x' are error-prone.
        for (a, b) in c.digram_freqs.keys() {
            for i in 0..20 {
                model.observe(*a, *b, 150.0);
                let err_prone = *a == 'x' || *b == 'x';
                model.record_attempt(*a, *b, !(err_prone && i % 4 == 0)); // 25% errors on x
            }
        }
        let focus = c.focus_pairs(&model, FOCUS_K);
        assert!(
            focus.iter().any(|((a, b), _)| *a == 'x' || *b == 'x'),
            "x pairs missing from focus: {focus:?}"
        );

        let mut rng = rand::rng();
        let mut x_lesson = 0;
        let mut x_uniform = 0;
        for _ in 0..50 {
            x_lesson += c.generate_lesson(&model, 10, &mut rng).matches('x').count();
            let uniform: Vec<&&str> = c.words.choose_multiple(&mut rng, 10).collect();
            x_uniform += uniform.iter().filter(|w| w.contains('x')).count();
        }
        assert!(
            x_lesson > x_uniform * 2,
            "lesson 'x' count {} vs uniform {}",
            x_lesson,
            x_uniform
        );
    }
}
