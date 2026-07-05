//! Word corpus, population digram frequencies, and adaptive lesson generation.

use crate::model::Model;
use rand::Rng;
use rand::seq::IndexedRandom;
use std::collections::HashMap;

/// Frequency-ranked English word list (google-10000-english).
const RAW_WORDS: &str = include_str!("../assets/words.txt");

/// Probability of picking a uniformly random word (exploration floor) instead
/// of a difficulty-weighted one, keeping coverage across all pairs.
const EXPLORATION_P: f64 = 0.3;
/// Uncertainty boost: pairs with few observations get their sampling weight
/// inflated by up to this factor.
const UNCERTAINTY_BOOST: f64 = 1.5;

pub struct Corpus {
    pub words: Vec<&'static str>,
    /// Zipf-weighted population digram frequencies (letters only).
    pub digram_freqs: HashMap<(char, char), f64>,
}

impl Corpus {
    pub fn load() -> Self {
        let words: Vec<&'static str> = RAW_WORDS
            .lines()
            .map(str::trim)
            .filter(|w| w.len() >= 2 && w.chars().all(|c| c.is_ascii_lowercase()))
            .collect();

        // The list is frequency-ranked; approximate word frequency by Zipf's
        // law (1/rank) to build a population digram distribution.
        let mut digram_freqs: HashMap<(char, char), f64> = HashMap::new();
        for (rank, word) in words.iter().enumerate() {
            let w = 1.0 / (rank as f64 + 1.0);
            let chars: Vec<char> = word.chars().collect();
            for pair in chars.windows(2) {
                *digram_freqs.entry((pair[0], pair[1])).or_insert(0.0) += w;
            }
        }
        let total: f64 = digram_freqs.values().sum();
        for v in digram_freqs.values_mut() {
            *v /= total;
        }

        Corpus { words, digram_freqs }
    }

    /// Difficulty score of a word: mean sampling weight of its digrams, where
    /// weight grows with estimated intrinsic pair cost and with uncertainty.
    fn word_score(&self, word: &str, model: &Model) -> f64 {
        let chars: Vec<char> = word.chars().collect();
        let mut sum = 0.0;
        let mut n = 0.0;
        for pair in chars.windows(2) {
            let s = model.pair_speed(pair[0], pair[1]);
            let count = model.pair_count(pair[0], pair[1]) as f64;
            let boost = 1.0 + (UNCERTAINTY_BOOST - 1.0) / (1.0 + count / 5.0);
            sum += s * boost;
            n += 1.0;
        }
        if n > 0.0 { sum / n } else { 0.0 }
    }

    /// Generate a lesson of `n_words`, continuously adaptive: words containing
    /// slow/uncertain digrams are sampled more often, with an exploration
    /// floor so nothing drops out of rotation.
    pub fn generate_lesson<R: Rng>(&self, model: &Model, n_words: usize, rng: &mut R) -> String {
        // Score a random candidate pool rather than all 10k words per lesson.
        let pool: Vec<&&str> = self
            .words
            .choose_multiple(rng, 400.min(self.words.len()))
            .collect();
        let scores: Vec<f64> = pool.iter().map(|w| self.word_score(w, model)).collect();
        let mean = scores.iter().sum::<f64>() / scores.len() as f64;

        // Softmax-ish weights relative to the mean difficulty.
        let weights: Vec<f64> = scores
            .iter()
            .map(|s| ((s - mean) / 40.0).exp().clamp(0.05, 20.0))
            .collect();
        let total_w: f64 = weights.iter().sum();

        let mut out: Vec<&str> = Vec::with_capacity(n_words);
        while out.len() < n_words {
            if rng.random::<f64>() < EXPLORATION_P {
                out.push(self.words.choose(rng).unwrap());
                continue;
            }
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
            q_adaptive += c
                .generate_lesson(&model, 10, &mut rng)
                .matches('q')
                .count();
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
