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

struct App {
    corpus: Corpus,
    model: Model,
    session: Session,
    rng: rand::rngs::ThreadRng,
    dirty: bool,
}

impl App {
    fn new() -> Self {
        let corpus = Corpus::load();
        let model = load_model();
        let mut rng = rand::rng();
        let session = Session::new(corpus.generate_lesson(&model, WORDS_PER_LESSON, &mut rng));
        App { corpus, model, session, rng, dirty: true }
    }

    fn next_lesson(&mut self) {
        self.model.recenter_biases();
        save_model(&self.model);
        self.session = Session::new(self.corpus.generate_lesson(
            &self.model,
            WORDS_PER_LESSON,
            &mut self.rng,
        ));
    }

    /// Returns false when the app should exit.
    fn handle_event(&mut self, ev: Event) -> bool {
        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
                self.dirty = true;
                match key.code {
                    KeyCode::Esc => return false,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return false;
                    }
                    KeyCode::Tab => self.next_lesson(),
                    KeyCode::Backspace => self.session.handle_backspace(),
                    KeyCode::Char(c) => {
                        self.session.handle_char(c, &mut self.model);
                        if self.session.done() {
                            self.next_lesson();
                        }
                    }
                    _ => self.dirty = false,
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
    let mut terminal = ratatui::init();

    let result = (|| -> std::io::Result<()> {
        loop {
            // Draw only when state changed; ratatui's buffer diff then writes
            // only the cells that differ, avoiding full-screen repaints.
            if app.dirty {
                terminal.draw(|f| ui::draw(f, &app.session, &app.model, &app.corpus))?;
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
