//! Explaining a run that was held back, as information rather than as an error.
//!
//! "Photos is open, so this backup didn't run" is not a failure to dig into, it
//! is a fact with a next step. A [`Notice`] lays that out: what happened, what
//! is in the way, and what to do. The one-line summary of the same event goes
//! to `vivo status` and `on-failure` hooks (see [`RunError`]).

use std::fmt;

use colored::Colorize;

/// Widest a notice is drawn, however wide the terminal.
const MAX_WIDTH: usize = 100;
const INDENT: &str = "    ";

/// A headline and labelled rows, e.g.
///
/// ```text
/// [i] Backup of 'photos' did not run.
///
///     Blocked by   Photos is running
///     Check        ! pgrep -x Photos (a `before` line in your config)
///     To continue  Quit Photos, then run `vivo photos` again
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    headline: String,
    rows: Vec<(&'static str, String)>,
}

impl Notice {
    pub fn new(headline: impl Into<String>) -> Self {
        Notice { headline: headline.into(), rows: Vec::new() }
    }

    /// Adds a row. A value with several lines (`\n`) keeps them aligned under the label.
    pub fn row(mut self, label: &'static str, value: impl Into<String>) -> Self {
        self.rows.push((label, value.into()));
        self
    }

    /// The rows as `(label cell, text)` lines, wrapped to `width` columns with
    /// continuation lines left under the text. The label cell is empty after a
    /// row's first line.
    fn layout(&self, width: usize) -> Vec<(String, String)> {
        let label_width = self.rows.iter().map(|(label, _)| label.chars().count()).max().unwrap_or(0);
        let text_width = width.saturating_sub(INDENT.len() + label_width + 2).max(20);

        let mut lines = Vec::new();
        for (label, value) in &self.rows {
            let mut first = true;
            for text in value.lines().flat_map(|l| wrap(l, text_width)) {
                let cell = if first { format!("{label:<label_width$}") } else { " ".repeat(label_width) };
                lines.push((cell, text));
                first = false;
            }
        }
        lines
    }

    /// The notice as plain text, `width` columns wide: what [`print`](Self::print)
    /// draws, without colour.
    pub fn render(&self, width: usize) -> String {
        let mut out = format!("[i] {}\n", self.headline);
        if !self.rows.is_empty() {
            out.push('\n');
        }
        for (label, text) in self.layout(width) {
            out.push_str(&format!("{INDENT}{label}  {text}\n"));
        }
        out
    }

    pub fn print(&self) {
        let width = crossterm::terminal::size().map_or(MAX_WIDTH, |(w, _)| usize::from(w)).min(MAX_WIDTH);
        println!();
        crate::ui::info(&self.headline.bold().to_string());
        let lines = self.layout(width);
        if !lines.is_empty() {
            println!();
        }
        for (label, text) in lines {
            println!("{INDENT}{}  {text}", label.cyan());
        }
        println!();
    }
}

/// Breaks `text` into lines of at most `width` characters at spaces. A word
/// longer than the width (a long path) stays whole on its own line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let fits = line.is_empty() || line.chars().count() + 1 + word.chars().count() <= width;
        if !fits {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines.push(line);
    lines
}

/// Why a run stopped: a one-line `summary` (what `vivo status` and `on-failure`
/// hooks show) and, when there is more to say, a [`Notice`] for the console.
/// Anything that is simply broken has no notice and is printed as an error.
#[derive(Debug, Clone, PartialEq)]
pub struct RunError {
    pub summary: String,
    pub notice: Option<Notice>,
}

impl RunError {
    pub fn with_notice(summary: impl Into<String>, notice: Notice) -> Self {
        RunError { summary: summary.into(), notice: Some(notice) }
    }

    /// Tells the user: the notice if there is one, otherwise `error: <summary>`.
    pub fn present(&self) {
        match &self.notice {
            Some(notice) => notice.print(),
            None => eprintln!("error: {}", self.summary),
        }
    }
}

impl From<String> for RunError {
    fn from(summary: String) -> Self {
        RunError { summary, notice: None }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Notice {
        Notice::new("Backup of 'photos' did not run.")
            .row("Blocked by", "Photos is running")
            .row("Check", "! pgrep -x Photos (a `before` line in your config)")
            .row("To continue", "Quit Photos, then run `vivo photos` again")
    }

    #[test]
    fn renders_a_headline_and_aligned_rows() {
        assert_eq!(
            sample().render(100),
            "[i] Backup of 'photos' did not run.\n\
             \n\
             \x20   Blocked by   Photos is running\n\
             \x20   Check        ! pgrep -x Photos (a `before` line in your config)\n\
             \x20   To continue  Quit Photos, then run `vivo photos` again\n"
        );
    }

    #[test]
    fn a_notice_without_rows_is_just_the_headline() {
        assert_eq!(Notice::new("Nothing to say.").render(80), "[i] Nothing to say.\n");
    }

    #[test]
    fn long_values_wrap_under_their_own_text() {
        let notice = Notice::new("Wrapped.").row("To continue", "one two three four five six seven eight nine ten eleven twelve");
        let text = notice.render(40);
        let lines: Vec<&str> = text.lines().skip(2).collect();
        assert!(lines.len() >= 3, "{text}");
        assert!(lines[0].starts_with("    To continue  one two"), "{text}");
        for line in &lines[1..] {
            assert!(line.starts_with("                 "), "continuation lines sit under the text: {text:?}");
        }
        assert!(lines.iter().all(|l| l.chars().count() <= 40), "{text}");
    }

    #[test]
    fn a_word_longer_than_the_width_is_not_split() {
        let path = "/Users/someone/Pictures/Photos Library.photoslibrary/originals/0/AAAA-BBBB-CCCC-DDDD-EEEE.heic";
        let text = Notice::new("Long.").row("Blocked by", format!("cannot read {}", path.replace(' ', "_"))).render(30);
        assert!(text.contains("/Users/someone/Pictures/Photos_Library.photoslibrary/originals/0/AAAA-BBBB-CCCC-DDDD-EEEE.heic"), "{text}");
    }

    #[test]
    fn multi_line_values_stay_aligned() {
        let text = Notice::new("Several.").row("Blocked by", "first thing\nsecond thing").row("To continue", "do it").render(100);
        assert!(text.contains("    Blocked by   first thing\n                 second thing\n    To continue  do it\n"), "{text}");
    }

    #[test]
    fn run_error_shows_its_summary_and_converts_from_a_string() {
        let plain: RunError = "restic failed".to_string().into();
        assert_eq!(plain.to_string(), "restic failed");
        assert!(plain.notice.is_none());
        let rich = RunError::with_notice("Photos is running, so nothing was backed up", sample());
        assert_eq!(rich.to_string(), "Photos is running, so nothing was backed up");
        assert_eq!(rich.notice, Some(sample()));
    }
}
