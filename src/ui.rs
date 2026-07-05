//! Composable UI widgets. Each component implements `Widget` so it can be
//! rendered into any `Rect`, rearranged, or reused independently. Rendering
//! goes through ratatui's double-buffered diff, so only changed cells are
//! written to the terminal (no flicker).

use crate::corpus::Corpus;
use crate::model::Model;
use crate::session::Session;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

/// Map a speed in ms to a heatmap color (fast=green .. slow=red).
fn speed_color(ms: f64) -> Color {
    let t = ((ms - 120.0) / 200.0).clamp(0.0, 1.0);
    Color::Rgb((255.0 * t) as u8, (200.0 * (1.0 - t)) as u8 + 55, 40)
}

/// Map an accuracy in [0,1] to a color (>=99% green .. <=90% red).
fn accuracy_color(acc: f64) -> Color {
    let t = ((0.99 - acc) / 0.09).clamp(0.0, 1.0);
    Color::Rgb((255.0 * t) as u8, (200.0 * (1.0 - t)) as u8 + 55, 40)
}

/// Convert a per-keystroke interval in ms to WPM (5 chars per word).
fn ms_to_wpm(ms: f64) -> f64 {
    60_000.0 / (ms.max(1.0) * 5.0)
}

/// Top-level frame composition: lays out and renders all components.
pub fn draw(
    frame: &mut Frame,
    session: &Session,
    model: &Model,
    corpus: &Corpus,
    delta_wpm: f64,
    delta_acc: f64,
) {
    let [
        header,
        _,
        lesson,
        _,
        errors,
        focus,
        _,
        heatmap,
        _,
        biases,
        _,
        footer,
    ] = Layout::vertical([
        Constraint::Length(2), // header (current stats + per-lesson deltas)
        Constraint::Length(1),
        Constraint::Length(4), // lesson (wraps)
        Constraint::Length(1),
        Constraint::Length(1), // error readout
        Constraint::Length(1), // focus pairs
        Constraint::Length(1),
        Constraint::Length(4), // heatmap (keys, wpm, accuracy)
        Constraint::Length(1),
        Constraint::Length(2), // biases
        Constraint::Fill(1),
        Constraint::Length(1), // footer
    ])
    .horizontal_margin(1)
    .areas(frame.area());

    frame.render_widget(
        StatsBar {
            session,
            model,
            corpus,
            delta_wpm,
            delta_acc,
        },
        header,
    );
    frame.render_widget(LessonText { session }, lesson);
    frame.render_widget(ErrorBar { session }, errors);
    frame.render_widget(FocusBar { model, corpus }, focus);
    frame.render_widget(KeyHeatmap { model, corpus }, heatmap);
    frame.render_widget(BiasReadout { model }, biases);
    frame.render_widget(Footer, footer);
}

/// Real-time error feedback: what kind of mistake just happened, plus
/// session totals by kind.
pub struct ErrorBar<'a> {
    pub session: &'a Session,
}

impl Widget for ErrorBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let s = self.session;
        let mut spans = vec![Span::styled("errors: ", Style::new().fg(Color::DarkGray))];
        spans.push(Span::raw(format!(
            "extra {}  skipped {}  typo {}  swap {}   ",
            s.insertions, s.omissions, s.substitutions, s.reversals
        )));
        if let Some(ev) = &s.last_error {
            let desc = match ev.kind {
                crate::session::ErrorKind::Insertion => {
                    format!("last: extra '{}' before '{}'", ev.got, ev.expected)
                }
                crate::session::ErrorKind::Omission => {
                    format!("last: skipped '{}'", ev.expected)
                }
                crate::session::ErrorKind::Substitution => {
                    format!("last: typo '{}' for '{}'", ev.got, ev.expected)
                }
                crate::session::ErrorKind::Reversal => {
                    format!("last: swapped '{}{}'", ev.expected, ev.got)
                }
            };
            spans.push(Span::styled(desc, Style::new().fg(Color::Yellow)));
        }
        Paragraph::new(Line::from(spans)).render(area, buf);
    }
}

/// The digrams currently being targeted by the lesson scheduler, with their
/// intrinsic (bias-invariant) speeds.
pub struct FocusBar<'a> {
    pub model: &'a Model,
    pub corpus: &'a Corpus,
}

impl Widget for FocusBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let focus = self.corpus.focus_pairs(self.model, crate::corpus::FOCUS_K);
        let mut spans = vec![Span::styled("focus: ", Style::new().fg(Color::DarkGray))];
        if focus.is_empty() {
            spans.push(Span::styled(
                "(none yet — keep typing)",
                Style::new().fg(Color::DarkGray),
            ));
        }
        for ((a, b), _) in &focus {
            let ms = self.model.pair_speed(*a, *b);
            spans.push(Span::styled(
                format!("{a}{b}"),
                Style::new().fg(speed_color(ms)),
            ));
            spans.push(Span::styled(
                format!(" {:.0}wpm   ", ms_to_wpm(ms)),
                Style::new().fg(Color::DarkGray),
            ));
        }
        Paragraph::new(Line::from(spans)).render(area, buf);
    }
}

/// Header: current normalized/raw WPM and accuracy, plus the change in
/// normalized scores produced by the last completed lesson.
pub struct StatsBar<'a> {
    pub session: &'a Session,
    pub model: &'a Model,
    pub corpus: &'a Corpus,
    pub delta_wpm: f64,
    pub delta_acc: f64,
}

/// Column widths shared by the stats row and the delta row so the two stay
/// aligned (e.g. `wpm_val` and `wpm_delta` line up) without needing to keep
/// two separate arrays in sync by hand.
const STATS_BAR_COLUMNS: [Constraint; 11] = [
    Constraint::Length(8), // title
    Constraint::Length(5), // "  wpm"
    Constraint::Length(5), // wpm value
    Constraint::Length(6), // "   raw"
    Constraint::Length(5), // raw wpm value
    Constraint::Length(6), // "   acc"
    Constraint::Length(6), // acc value (% included)
    Constraint::Length(6), // "   raw"
    Constraint::Length(6), // raw acc value (% included)
    Constraint::Length(6), // "   obs"
    Constraint::Min(0),    // obs value
];

impl Widget for StatsBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [row1, row2] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

        let [
            title,
            wpm_label,
            wpm_val,
            raw_wpm_label,
            raw_wpm_val,
            acc_label,
            acc_val,
            raw_acc_label,
            raw_acc_val,
            obs_label,
            obs_val,
        ] = Layout::horizontal(STATS_BAR_COLUMNS).areas(row1);

        let wpm = self.model.normalized_wpm(&self.corpus.digram_freqs);
        let acc = self.model.normalized_accuracy(&self.corpus.digram_freqs);

        // Row 1: current stats.
        Paragraph::new(Line::from(Span::styled(
            " clank ",
            Style::new().bold().fg(Color::Cyan),
        )))
        .render(title, buf);
        Paragraph::new("  wpm").render(wpm_label, buf);
        Paragraph::new(format!("{:.1}", wpm))
            .alignment(Alignment::Right)
            .render(wpm_val, buf);
        Paragraph::new("   raw").render(raw_wpm_label, buf);
        Paragraph::new(format!("{:.1}", self.session.raw_wpm()))
            .alignment(Alignment::Right)
            .render(raw_wpm_val, buf);
        Paragraph::new("   acc").render(acc_label, buf);
        Paragraph::new(format!("{:.1}%", 100.0 * acc))
            .alignment(Alignment::Right)
            .render(acc_val, buf);
        Paragraph::new("   raw").render(raw_acc_label, buf);
        Paragraph::new(format!("{:.1}%", 100.0 * self.session.raw_accuracy()))
            .alignment(Alignment::Right)
            .render(raw_acc_val, buf);
        Paragraph::new("   obs").render(obs_label, buf);
        Paragraph::new(format!("{}", self.model.total_obs)).render(obs_val, buf);

        // Row 2: per-lesson deltas, aligned under normalized WPM and accuracy.
        let [_, _, wpm_delta, _, _, _, acc_delta, _, _, _, _] =
            Layout::horizontal(STATS_BAR_COLUMNS).areas(row2);

        let wpm_color = if self.delta_wpm >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };
        let acc_color = if self.delta_acc >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };

        Paragraph::new(Span::styled(
            format!("{:+.2}", self.delta_wpm),
            Style::new().fg(wpm_color),
        ))
        .alignment(Alignment::Right)
        .render(wpm_delta, buf);
        Paragraph::new(Span::styled(
            format!("{:+.2}%", 100.0 * self.delta_acc),
            Style::new().fg(acc_color),
        ))
        .alignment(Alignment::Right)
        .render(acc_delta, buf);
    }
}

/// The lesson text with per-character feedback and a cursor highlight.
pub struct LessonText<'a> {
    pub session: &'a Session,
}

impl Widget for LessonText<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let s = self.session;
        let spans: Vec<Span> = s
            .lesson
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let style = if i < s.typed.len() {
                    if s.typed[i] {
                        Style::new().fg(Color::Green)
                    } else {
                        Style::new().fg(Color::Red).underlined()
                    }
                } else if i == s.pos {
                    if s.pending.is_some() {
                        // Unresolved mismatch: cursor turns red.
                        Style::new().fg(Color::White).bg(Color::Red)
                    } else {
                        Style::new().fg(Color::Black).bg(Color::White)
                    }
                } else {
                    Style::new().fg(Color::DarkGray)
                };
                Span::styled(c.to_string(), style)
            })
            .collect();
        Paragraph::new(Line::from(spans))
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

/// a-z heatmap of population-weighted intrinsic key speeds (as WPM) and
/// per-key accuracy.
pub struct KeyHeatmap<'a> {
    pub model: &'a Model,
    pub corpus: &'a Corpus,
}

impl Widget for KeyHeatmap<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from(Span::styled(
            "per key: wpm and accuracy % (green=good, red=needs work):",
            Style::new().fg(Color::DarkGray),
        ));
        let mut keys: Vec<Span> = Vec::with_capacity(27);
        let mut wpms: Vec<Span> = Vec::with_capacity(27);
        let mut accs: Vec<Span> = Vec::with_capacity(27);
        keys.push(Span::styled("     ", Style::new().fg(Color::DarkGray)));
        wpms.push(Span::styled("wpm  ", Style::new().fg(Color::DarkGray)));
        accs.push(Span::styled("acc  ", Style::new().fg(Color::DarkGray)));
        for c in 'a'..='z' {
            let ms = self.model.key_speed(c, &self.corpus.digram_freqs);
            let acc = self.model.key_accuracy(c, &self.corpus.digram_freqs);
            keys.push(Span::styled(
                format!("{c}   "),
                Style::new().fg(speed_color(ms)),
            ));
            wpms.push(Span::styled(
                format!("{:<4.0}", ms_to_wpm(ms)),
                Style::new().fg(speed_color(ms)),
            ));
            accs.push(Span::styled(
                format!("{:<4.0}", 100.0 * acc),
                Style::new().fg(accuracy_color(acc)),
            ));
        }
        Paragraph::new(vec![
            title,
            Line::from(keys),
            Line::from(wpms),
            Line::from(accs),
        ])
        .render(area, buf);
    }
}

/// Largest per-key biases: the "late/early" tendencies the model isolates.
pub struct BiasReadout<'a> {
    pub model: &'a Model,
}

impl Widget for BiasReadout<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from(Span::styled(
            "largest key biases, ms (+late start / -early start):",
            Style::new().fg(Color::DarkGray),
        ));
        let mut biased: Vec<(char, f64)> =
            ('a'..='z').map(|c| (c, self.model.key_bias(c))).collect();
        biased.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
        let spans: Vec<Span> = biased
            .iter()
            .take(8)
            .map(|(c, b)| Span::raw(format!("{c}:{b:+.0}  ")))
            .collect();
        Paragraph::new(vec![title, Line::from(spans)]).render(area, buf);
    }
}

/// Key hints.
pub struct Footer;

impl Widget for Footer {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(Span::styled(
            "esc quit  ·  tab new lesson",
            Style::new().fg(Color::DarkGray),
        ))
        .render(area, buf);
    }
}
