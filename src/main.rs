mod corpus;
mod model;

use corpus::Corpus;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{cursor, execute, queue};
use model::{Model, PAUSE_THRESHOLD_MS};
use std::io::{Write, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WORDS_PER_LESSON: usize = 10;

fn state_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("clank");
    std::fs::create_dir_all(&p).ok();
    p.push("model.json");
    p
}

fn load_model() -> Model {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_model(model: &Model) {
    if let Ok(json) = serde_json::to_string(model) {
        std::fs::write(state_path(), json).ok();
    }
}

/// Map a speed in ms to a heatmap color (fast=green .. slow=red).
fn speed_color(ms: f64) -> Color {
    let t = ((ms - 120.0) / 200.0).clamp(0.0, 1.0);
    Color::Rgb {
        r: (255.0 * t) as u8,
        g: (200.0 * (1.0 - t)) as u8 + 55,
        b: 40,
    }
}

struct Session {
    lesson: Vec<char>,
    typed: Vec<bool>, // correctness of each typed position
    pos: usize,
    last_key: Option<(char, Instant)>,
    // Session-raw stats (actual keystrokes, unnormalized).
    session_chars: u64,
    session_ms: f64,
    errors: u64,
}

impl Session {
    fn new(lesson: String) -> Self {
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

    fn done(&self) -> bool {
        self.pos >= self.lesson.len()
    }

    fn handle_char(&mut self, c: char, model: &mut Model) {
        if self.done() {
            return;
        }
        let expected = self.lesson[self.pos];
        let now = Instant::now();
        let correct = c == expected;

        if correct {
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

    fn handle_backspace(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
            self.typed.pop();
            self.last_key = None; // exclude intervals adjacent to corrections
        }
    }

    fn raw_wpm(&self) -> f64 {
        if self.session_ms <= 0.0 {
            return 0.0;
        }
        (self.session_chars as f64 / 5.0) / (self.session_ms / 60_000.0)
    }
}

fn render(
    out: &mut impl Write,
    session: &Session,
    model: &Model,
    corpus: &Corpus,
) -> std::io::Result<()> {
    queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;

    // Header: normalized WPM (population-weighted, difficulty-invariant) + raw.
    queue!(
        out,
        SetForegroundColor(Color::Cyan),
        Print(format!(
            " clank  |  wpm {:5.1} (normalized)   raw {:5.1}   errors {}   obs {}",
            model.normalized_wpm(&corpus.digram_freqs),
            session.raw_wpm(),
            session.errors,
            model.total_obs,
        )),
        ResetColor,
        cursor::MoveTo(1, 2)
    )?;

    // Lesson text with per-char feedback.
    for (i, &c) in session.lesson.iter().enumerate() {
        if i < session.typed.len() {
            let color = if session.typed[i] { Color::Green } else { Color::Red };
            queue!(out, SetForegroundColor(color), Print(c), ResetColor)?;
        } else if i == session.pos {
            queue!(
                out,
                SetForegroundColor(Color::Black),
                crossterm::style::SetBackgroundColor(Color::White),
                Print(c),
                ResetColor
            )?;
        } else {
            queue!(out, SetForegroundColor(Color::DarkGrey), Print(c), ResetColor)?;
        }
    }

    // Per-key speed heatmap (population-weighted intrinsic speeds).
    queue!(
        out,
        cursor::MoveTo(0, 4),
        SetForegroundColor(Color::DarkGrey),
        Print(" key speed, ms/transition (green=fast, red=slow):"),
        ResetColor,
    )?;
    for c in 'a'..='z' {
        let col = 1 + (c as u16 - 'a' as u16) * 4;
        let ms = model.key_speed(c, &corpus.digram_freqs);
        queue!(
            out,
            cursor::MoveTo(col, 5),
            SetForegroundColor(speed_color(ms)),
            Print(c),
            cursor::MoveTo(col, 6),
            Print(format!("{:3.0}", ms)),
            ResetColor
        )?;
    }

    // Per-key bias (the "late/early" tendency the model isolates).
    queue!(
        out,
        cursor::MoveTo(0, 8),
        SetForegroundColor(Color::DarkGrey),
        Print(" largest key biases, ms (+late start / -early start):"),
        ResetColor,
        cursor::MoveTo(1, 9)
    )?;
    let mut biased: Vec<(char, f64)> = ('a'..='z').map(|c| (c, model.key_bias(c))).collect();
    biased.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
    for (c, b) in biased.iter().take(8) {
        queue!(out, Print(format!("{}:{:+.0}  ", c, b)))?;
    }

    queue!(
        out,
        cursor::MoveTo(0, 11),
        SetForegroundColor(Color::DarkGrey),
        Print(" esc quit  ·  tab new lesson"),
        ResetColor
    )?;

    out.flush()
}

fn main() -> std::io::Result<()> {
    let corpus = Corpus::load();
    let mut model = load_model();
    let mut rng = rand::rng();
    let mut session = Session::new(corpus.generate_lesson(&model, WORDS_PER_LESSON, &mut rng));

    let mut out = stdout();
    terminal::enable_raw_mode()?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;

    let result = (|| -> std::io::Result<()> {
        render(&mut out, &session, &model, &corpus)?;
        loop {
            if !event::poll(Duration::from_millis(50))? {
                continue;
            }
            let Event::Key(KeyEvent { code, kind, modifiers, .. }) = event::read()? else {
                continue;
            };
            if kind != KeyEventKind::Press && kind != KeyEventKind::Repeat {
                continue;
            }
            match code {
                KeyCode::Esc => break,
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Tab => {
                    model.recenter_biases();
                    session =
                        Session::new(corpus.generate_lesson(&model, WORDS_PER_LESSON, &mut rng));
                }
                KeyCode::Backspace => session.handle_backspace(),
                KeyCode::Char(c) => {
                    session.handle_char(c, &mut model);
                    if session.done() {
                        model.recenter_biases();
                        save_model(&model);
                        session = Session::new(corpus.generate_lesson(
                            &model,
                            WORDS_PER_LESSON,
                            &mut rng,
                        ));
                    }
                }
                _ => {}
            }
            render(&mut out, &session, &model, &corpus)?;
        }
        Ok(())
    })();

    execute!(out, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    model.recenter_biases();
    save_model(&model);
    result
}
