//! Typing session state: lesson text, cursor, correctness, error
//! classification, and raw stats.
//!
//! Error handling is "strict": the cursor does not advance past a position
//! until it is resolved, which is what makes real-time error classification
//! well-defined. A mismatched keystroke is classified with one-key lookahead
//! (expected `e`, next `n`, then `n2`):
//! - wrong `w`, then `e`  (w != n)  -> insertion (an extra letter was hit)
//! - wrong `w`, then `n`            -> substitution (typed `w` in place of `e`)
//! - wrong `w == n`, then `e`       -> reversal (typed `n` and `e` swapped)
//! - wrong `w == n`, then `n2`      -> omission (`e` was skipped)

use crate::model::{Model, PAUSE_THRESHOLD_MS};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// An extra letter was hit before the expected one.
    Insertion,
    /// The expected letter was skipped.
    Omission,
    /// A different letter was typed in place of the expected one.
    Substitution,
    /// Two adjacent letters were typed in swapped order.
    Reversal,
}

#[derive(Debug, Clone, Copy)]
pub struct ErrorEvent {
    pub kind: ErrorKind,
    /// The character that was actually typed.
    pub got: char,
    /// The character that was expected.
    pub expected: char,
}

pub struct Session {
    pub lesson: Vec<char>,
    pub typed: Vec<bool>, // correctness of each resolved lesson position
    pub pos: usize,
    /// A mismatched keystroke awaiting classification by the next key.
    pub pending: Option<char>,
    pub last_error: Option<ErrorEvent>,
    pub insertions: u64,
    pub omissions: u64,
    pub substitutions: u64,
    pub reversals: u64,
    /// Correctly typed characters (attempt successes).
    correct: u64,
    last_key: Option<(char, Instant)>,
    // Session-raw stats (actual keystrokes, unnormalized).
    session_chars: u64,
    session_ms: f64,
}

impl Session {
    pub fn new(lesson: String) -> Self {
        let chars: Vec<char> = lesson.chars().collect();
        let n = chars.len();
        Session {
            lesson: chars,
            typed: Vec::with_capacity(n),
            pos: 0,
            pending: None,
            last_error: None,
            insertions: 0,
            omissions: 0,
            substitutions: 0,
            reversals: 0,
            correct: 0,
            last_key: None,
            session_chars: 0,
            session_ms: 0.0,
        }
    }

    pub fn done(&self) -> bool {
        self.pos >= self.lesson.len()
    }

    pub fn errors(&self) -> u64 {
        self.insertions + self.omissions + self.substitutions + self.reversals
    }

    /// Raw session accuracy in [0, 1]: correct keystrokes over attempts.
    pub fn raw_accuracy(&self) -> f64 {
        let attempts = self.correct + self.errors();
        if attempts == 0 {
            return 1.0;
        }
        self.correct as f64 / attempts as f64
    }

    pub fn handle_char(&mut self, c: char, model: &mut Model) {
        if self.done() {
            return;
        }
        let expected = self.lesson[self.pos];
        let next = self.lesson.get(self.pos + 1).copied();
        let now = Instant::now();

        if let Some(wrong) = self.pending {
            let next2 = self.lesson.get(self.pos + 2).copied();
            if Some(wrong) == next {
                // The mismatch matched the NEXT expected char: ambiguous
                // between reversal, omission, and substitution until now.
                if c == expected {
                    // w == n then e: adjacent letters typed in swapped order.
                    self.record_error(ErrorKind::Reversal, wrong, expected, model);
                    self.pending = None;
                    self.typed.push(false);
                    self.typed.push(false);
                    self.pos += 2;
                    // `c` is a real keystroke; the timing chain restarts here.
                    self.last_key = Some((c, now));
                } else if next2 == Some(c) {
                    // w == n then n2: `e` was genuinely skipped.
                    self.record_error(ErrorKind::Omission, wrong, expected, model);
                    self.pending = None;
                    self.typed.push(false);
                    self.typed.push(true); // `n` was typed correctly (as `w`)
                    self.correct += 1;
                    self.pos += 2;
                    self.accept(c, now, model);
                } else if next == Some(c) {
                    // w == n then n again: `w` replaced `e`.
                    self.record_error(ErrorKind::Substitution, wrong, expected, model);
                    self.pending = None;
                    self.typed.push(false);
                    self.pos += 1;
                    self.accept(c, now, model);
                } else {
                    // Still unresolved; count the previous mismatch and keep
                    // waiting on the new key.
                    self.record_error(ErrorKind::Substitution, wrong, expected, model);
                    self.pending = Some(c);
                }
            } else if c == expected {
                // wrong, then expected: the wrong char was an extra keystroke.
                self.record_error(ErrorKind::Insertion, wrong, expected, model);
                self.pending = None;
                self.accept(expected, now, model);
            } else if next == Some(c) {
                // wrong, then char after expected: wrong char replaced expected.
                self.record_error(ErrorKind::Substitution, wrong, expected, model);
                self.pending = None;
                self.typed.push(false);
                self.pos += 1;
                self.accept(c, now, model);
            } else {
                // Still unresolved; count the previous mismatch as a
                // substitution attempt and keep waiting on the new key.
                self.record_error(ErrorKind::Substitution, wrong, expected, model);
                self.pending = Some(c);
            }
            return;
        }

        if c == expected {
            self.accept(expected, now, model);
        } else {
            // Any mismatch — including one matching the next expected char —
            // is ambiguous until the following key; classify then.
            self.pending = Some(c);
            self.last_key = None; // exclude intervals adjacent to errors
        }
    }

    /// Accept `c` as the correct char at the cursor. Records timing and an
    /// accuracy success only when the keystroke chain is intact.
    fn accept(&mut self, c: char, now: Instant, model: &mut Model) {
        if let Some((prev, t0)) = self.last_key {
            let dt = now.duration_since(t0).as_secs_f64() * 1000.0;
            if prev.is_ascii_lowercase() && c.is_ascii_lowercase() {
                if model.observe(prev, c, dt) {
                    self.session_chars += 1;
                    self.session_ms += dt;
                }
                model.record_attempt(prev, c, true);
            } else if dt < PAUSE_THRESHOLD_MS {
                self.session_chars += 1;
                self.session_ms += dt;
            }
        }
        self.correct += 1;
        self.last_key = Some((c, now));
        self.typed.push(true);
        self.pos += 1;
    }

    fn record_error(&mut self, kind: ErrorKind, got: char, expected: char, model: &mut Model) {
        match kind {
            ErrorKind::Insertion => self.insertions += 1,
            ErrorKind::Omission => self.omissions += 1,
            ErrorKind::Substitution => self.substitutions += 1,
            ErrorKind::Reversal => self.reversals += 1,
        }
        self.last_error = Some(ErrorEvent { kind, got, expected });
        match kind {
            // A reversal is an ordering failure of the expected->next
            // transition itself (got == next here).
            ErrorKind::Reversal => {
                if expected.is_ascii_lowercase() && got.is_ascii_lowercase() {
                    model.record_attempt(expected, got, false);
                }
            }
            // Other errors count against the transition into the expected char.
            _ => {
                if self.pos > 0 {
                    let prev = self.lesson[self.pos - 1];
                    if prev.is_ascii_lowercase() && expected.is_ascii_lowercase() {
                        model.record_attempt(prev, expected, false);
                    }
                }
            }
        }
        self.last_key = None; // exclude intervals adjacent to errors
    }

    pub fn handle_backspace(&mut self) {
        if self.pending.is_some() {
            // The mismatched key was never committed; just clear it.
            self.pending = None;
            self.last_error = None;
            return;
        }
        if self.pos > 0 {
            self.pos -= 1;
            self.typed.pop();
            self.last_key = None; // exclude intervals adjacent to corrections
        }
    }

    pub fn raw_wpm(&self) -> f64 {
        if self.session_ms <= 0.0 {
            return 0.0;
        }
        (self.session_chars as f64 / 5.0) / (self.session_ms / 60_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(session: &mut Session, model: &mut Model, s: &str) {
        for c in s.chars() {
            session.handle_char(c, model);
        }
    }

    #[test]
    fn classifies_insertion() {
        let mut m = Model::default();
        let mut s = Session::new("cat".into());
        feed(&mut s, &mut m, "cxat"); // extra 'x' before 'a'
        assert_eq!(s.insertions, 1);
        assert_eq!(s.errors(), 1);
        assert!(s.done());
        assert_eq!(s.typed, vec![true, true, true]);
        assert_eq!(s.last_error.unwrap().kind, ErrorKind::Insertion);
    }

    #[test]
    fn classifies_omission() {
        let mut m = Model::default();
        let mut s = Session::new("cats".into());
        feed(&mut s, &mut m, "cts"); // skipped the 'a', kept going
        assert_eq!(s.omissions, 1);
        assert!(s.done());
        assert_eq!(s.typed, vec![true, false, true, true]);
        assert_eq!(s.last_error.unwrap().kind, ErrorKind::Omission);
    }

    #[test]
    fn classifies_reversal() {
        let mut m = Model::default();
        let mut s = Session::new("cat".into());
        feed(&mut s, &mut m, "cta"); // 'a' and 't' typed in swapped order
        assert_eq!(s.reversals, 1);
        assert_eq!(s.errors(), 1);
        assert!(s.done());
        assert_eq!(s.typed, vec![true, false, false]);
        assert_eq!(s.last_error.unwrap().kind, ErrorKind::Reversal);
    }

    #[test]
    fn next_char_mismatch_can_still_be_substitution() {
        let mut m = Model::default();
        // 't' typed in place of 'a' happens to equal the next expected char;
        // the repeated 't' disambiguates it as a substitution, not a skip.
        let mut s = Session::new("cat".into());
        feed(&mut s, &mut m, "ctt");
        assert_eq!(s.substitutions, 1);
        assert_eq!(s.omissions, 0);
        assert!(s.done());
        assert_eq!(s.typed, vec![true, false, true]);
        assert_eq!(s.last_error.unwrap().kind, ErrorKind::Substitution);
    }

    #[test]
    fn classifies_substitution() {
        let mut m = Model::default();
        let mut s = Session::new("cat".into());
        feed(&mut s, &mut m, "cxt"); // typed 'x' in place of 'a', moved on
        assert_eq!(s.substitutions, 1);
        assert!(s.done());
        assert_eq!(s.typed, vec![true, false, true]);
        assert_eq!(s.last_error.unwrap().kind, ErrorKind::Substitution);
    }

    #[test]
    fn backspace_clears_pending_mismatch() {
        let mut m = Model::default();
        let mut s = Session::new("cat".into());
        feed(&mut s, &mut m, "cq");
        assert!(s.pending.is_some());
        s.handle_backspace();
        assert!(s.pending.is_none());
        assert_eq!(s.pos, 1); // still at the 'a'
        feed(&mut s, &mut m, "at");
        assert!(s.done());
    }

    #[test]
    fn errors_feed_pair_accuracy() {
        let mut m = Model::default();
        let mut s = Session::new("cat".into());
        feed(&mut s, &mut m, "cxt"); // substitution at the c->a transition
        let acc_ca = m.pair_accuracy('c', 'a');
        let acc_at = m.pair_accuracy('a', 't');
        assert!(acc_ca < acc_at, "c->a should be less accurate ({acc_ca} vs {acc_at})");
        assert!(s.raw_accuracy() < 1.0);
    }
}
