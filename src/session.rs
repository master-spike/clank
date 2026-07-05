//! Typing session state: lesson text, cursor, correctness, raw stats.

use crate::model::{Model, PAUSE_THRESHOLD_MS};
use std::time::Instant;

pub struct Session {
    pub lesson: Vec<char>,
    pub typed: Vec<bool>, // correctness of each typed position
    pub pos: usize,
    last_key: Option<(char, Instant)>,
    // Session-raw stats (actual keystrokes, unnormalized).
    session_chars: u64,
    session_ms: f64,
    pub errors: u64,
}

impl Session {
    pub fn new(lesson: String) -> Self {
        let chars: Vec<char> = lesson.chars().collect();
        let n = chars.len();
        Session {
            lesson: chars,
            typed: Vec::with_capacity(n),
            pos: 0,
            last_key: None,
            session_chars: 0,
            session_ms: 0.0,
            errors: 0,
        }
    }

    pub fn done(&self) -> bool {
        self.pos >= self.lesson.len()
    }

    pub fn handle_char(&mut self, c: char, model: &mut Model) {
        if self.done() {
            return;
        }
        let expected = self.lesson[self.pos];
        let now = Instant::now();

        if c == expected {
            // Record interval only for letter->letter transitions with a
            // valid, correct predecessor (errors/pauses break the chain).
            if let Some((prev, t0)) = self.last_key {
                let dt = now.duration_since(t0).as_secs_f64() * 1000.0;
                if prev.is_ascii_lowercase() && expected.is_ascii_lowercase() {
                    if model.observe(prev, expected, dt) {
                        self.session_chars += 1;
                        self.session_ms += dt;
                    }
                } else if dt < PAUSE_THRESHOLD_MS {
                    self.session_chars += 1;
                    self.session_ms += dt;
                }
            }
            self.last_key = Some((expected, now));
            self.typed.push(true);
            self.pos += 1;
        } else {
            self.errors += 1;
            self.typed.push(false);
            self.pos += 1;
            self.last_key = None; // exclude intervals adjacent to errors
        }
    }

    pub fn handle_backspace(&mut self) {
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
