mod corpus;
mod model;
mod session;
mod ui;

use corpus::Corpus;
use model::Model;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use session::Session;
use std::path::PathBuf;
use std::time::Duration;
use ui::StatsView;

const WORDS_PER_LESSON: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Typing,
    Stats,
}

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
    let Ok(json) = serde_json::to_string(model) else {
        return;
    };
    // Write to a temp file and rename over the target so an interrupted write
    // can never truncate/corrupt an existing model (rename is atomic on the
    // same filesystem).
    let path = state_path();
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        std::fs::rename(&tmp, &path).ok();
    }
}

struct App {
    corpus: Corpus,
    model: Model,
    session: Session,
    rng: rand::rngs::ThreadRng,
    dirty: bool,
    last_delta_wpm: f64,
    last_delta_acc: f64,
    mode: AppMode,
    matrix_scroll: (u16, u16),
    stats_view: StatsView,
}

impl App {
    fn new() -> Self {
        let corpus = Corpus::load();
        let model = load_model();
        let mut rng = rand::rng();
        let session = Session::new(
            corpus.generate_lesson(&model, WORDS_PER_LESSON, &mut rng),
            &model,
            &corpus.digram_freqs,
        );
        App {
            corpus,
            model,
            session,
            rng,
            dirty: true,
            last_delta_wpm: 0.0,
            last_delta_acc: 0.0,
            mode: AppMode::Typing,
            matrix_scroll: (0, 0),
            stats_view: StatsView::Wpm,
        }
    }

    fn next_lesson(&mut self) {
        self.model.recenter_biases();
        save_model(&self.model);

        (self.last_delta_wpm, self.last_delta_acc) = self
            .session
            .normalized_deltas(&self.model, &self.corpus.digram_freqs);

        self.session = Session::new(
            self.corpus
                .generate_lesson(&self.model, WORDS_PER_LESSON, &mut self.rng),
            &self.model,
            &self.corpus.digram_freqs,
        );
    }

    /// Returns false when the app should exit.
    fn handle_event(&mut self, ev: Event) -> bool {
        match ev {
            Event::Key(key)
                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
            {
                // Global quit shortcuts work in every mode.
                if key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return false;
                }
                self.dirty = true;
                match self.mode {
                    AppMode::Stats => match key.code {
                        KeyCode::Char('<') | KeyCode::Char('>') => self.mode = AppMode::Typing,
                        KeyCode::Char('n') => self.stats_view = self.stats_view.next(),
                        KeyCode::Char('p') => self.stats_view = self.stats_view.prev(),
                        KeyCode::Left => self.matrix_scroll.0 = self.matrix_scroll.0.saturating_sub(1),
                        KeyCode::Right => self.matrix_scroll.0 = self.matrix_scroll.0.saturating_add(1),
                        KeyCode::Up => self.matrix_scroll.1 = self.matrix_scroll.1.saturating_sub(1),
                        KeyCode::Down => self.matrix_scroll.1 = self.matrix_scroll.1.saturating_add(1),
                        _ => self.dirty = false,
                    },
                    AppMode::Typing => match key.code {
                        KeyCode::Char('<') | KeyCode::Char('>') => self.mode = AppMode::Stats,
                        KeyCode::Tab => self.next_lesson(),
                        KeyCode::Backspace => self.session.handle_backspace(),
                        KeyCode::Char(c) => {
                            self.session.handle_char(c, &mut self.model);
                            if self.session.done() {
                                self.next_lesson();
                            }
                        }
                        _ => self.dirty = false,
                    },
                }
            }
            Event::Resize(_, _) => self.dirty = true,
            _ => {}
        }
        true
    }
}

fn main() -> std::io::Result<()> {
    let mut app = App::new();

    // Ensure the terminal is restored even if a later panic unwinds past the
    // normal cleanup path, so the user's shell isn't left in raw mode.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        default_hook(info);
    }));

    let mut terminal = ratatui::init();

    let result = (|| -> std::io::Result<()> {
        loop {
            // Draw only when state changed; ratatui's buffer diff then writes
            // only the cells that differ, avoiding full-screen repaints.
            if app.dirty {
                if app.mode == AppMode::Stats {
                    terminal.draw(|f| {
                        ui::draw_stats(f, &app.model, &app.corpus, app.matrix_scroll, app.stats_view)
                    })?;
                } else {
                    terminal.draw(|f| {
                        ui::draw(
                            f,
                            &app.session,
                            &app.model,
                            &app.corpus,
                            app.last_delta_wpm,
                            app.last_delta_acc,
                        )
                    })?;
                }
                app.dirty = false;
            }
            if event::poll(Duration::from_millis(100))? && !app.handle_event(event::read()?) {
                break;
            }
        }
        Ok(())
    })();

    ratatui::restore();
    app.model.recenter_biases();
    save_model(&app.model);
    result
}
