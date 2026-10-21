use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use super::app::{App, Pane, FIELD_COUNT};

// Palette. ANSI "bright black" (DarkGray) is nearly invisible on several
// popular dark themes, so text uses a fixed 256-color gray instead and
// DarkGray is kept for unfocused borders only.
const MUTED: Style = Style::new().fg(Color::Indexed(247));
const FAINT: Style = Style::new().fg(Color::Indexed(244)).add_modifier(Modifier::ITALIC);
const ACCENT: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
const BORDER_IDLE: Style = Style::new().fg(Color::Indexed(240));
const SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Yellow)
    .add_modifier(Modifier::BOLD);
// Selected row in a pane that doesn't have focus: still visible, but quieter.
const SELECTED_IDLE: Style = Style::new()
    .bg(Color::Indexed(238))
    .add_modifier(Modifier::BOLD);

fn selection(focused: bool) -> Style {
    if focused { SELECTED } else { SELECTED_IDLE }
}

fn header(focused: bool) -> Style {
    if focused { ACCENT } else { MUTED }
}

// Pre-padded to 18 chars (the longest label plus two spaces) — avoids a
// format! allocation per field per frame.
const FIELD_LABELS: [&str; FIELD_COUNT] = [
    "Name              ",
    "Description       ",
    "Repo              ",
    "Directory         ",
    "Exclude file      ",
    "Files from        ",
    "Schedule          ",
    "Password profile  ",
];

pub fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(outer[0]);

    draw_tasks(f, app, panes[0]);
    draw_task_detail(f, app, panes[1]);
    draw_help(f, app, outer[1]);
}

fn focused_border(focused: bool) -> Style {
    if focused { ACCENT } else { BORDER_IDLE }
}

fn draw_tasks(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .tasks
        .iter()
        .map(|t| {
            let label = if t.name == app.default_task {
                format!("{} *", t.name)
            } else {
                t.name.clone()
            };
            ListItem::new(label)
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Tasks ")
                .border_style(focused_border(app.focused_pane == Pane::Tasks)),
        )
        .highlight_style(selection(app.focused_pane == Pane::Tasks));

    let mut state = ListState::default();
    if !app.tasks.is_empty() {
        state.select(Some(app.selected_task));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_task_detail(f: &mut Frame, app: &App, area: Rect) {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.as_str())
        .unwrap_or("\u{2014}");
    let title = format!(" Task: {task_name} ");

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(focused_border(
            app.focused_pane == Pane::Fields
                || app.focused_pane == Pane::Remotes
                || app.focused_pane == Pane::Calls,
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let remotes_min = (1 + app.current_remotes().len()) as u16;
    let calls_min = (1 + app.current_calls().len()) as u16;
    let details = app.tasks.get(app.selected_task).map(|t| t.detail_lines()).unwrap_or_default();
    // A header line plus one per setting, or nothing when there are none.
    let details_height = if details.is_empty() { 0 } else { 1 + details.len() as u16 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(FIELD_COUNT as u16),
            Constraint::Length(details_height),
            Constraint::Min(remotes_min),
            Constraint::Min(calls_min),
        ])
        .split(inner);

    draw_task_fields(f, app, chunks[0]);
    draw_task_details(f, &details, chunks[1]);
    draw_remotes_list(f, app, chunks[2]);
    draw_calls_list(f, app, chunks[3]);
}

fn draw_task_fields(f: &mut Frame, app: &App, area: Rect) {
    let task = match app.tasks.get(app.selected_task) {
        Some(t) => t,
        None => return,
    };

    let in_fields = app.focused_pane == Pane::Fields;
    let label_style = MUTED;
    let highlight = selection(in_fields);

    let values: [Option<&str>; FIELD_COUNT] = [
        Some(task.name.as_str()),
        task.description.as_deref(),
        task.repo.as_deref(),
        task.directory.as_deref(),
        task.exclude_file.as_deref(),
        task.files_from.as_deref(),
        task.schedule.as_deref(),
        task.password_profile.as_deref(),
    ];

    let lines: Vec<Line> = FIELD_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let selected = i == app.selected_field;
            let value_span = match values[i] {
                Some(v) if !v.is_empty() => Span::raw(v.to_string()),
                _ => Span::styled("(none)", FAINT),
            };
            let line = Line::from(vec![Span::styled(*label, label_style), value_span]);
            if selected { line.style(highlight) } else { line }
        })
        .collect();

    f.render_widget(Paragraph::new(lines), area);
}

/// Settings that have no row of their own, read-only; `o` opens the config to change them.
fn draw_task_details(f: &mut Frame, details: &[String], area: Rect) {
    if details.is_empty() {
        return;
    }
    let mut lines = vec![Line::from(Span::styled("Also set (press o to edit the file):", MUTED))];
    lines.extend(details.iter().map(|d| Line::from(Span::raw(format!("  {d}")))));
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_remotes_list(f: &mut Frame, app: &App, area: Rect) {
    let remotes = app.current_remotes();
    let count = remotes.len();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header_style = header(app.focused_pane == Pane::Remotes);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("Remotes ({count}):"),
            header_style,
        )])),
        chunks[0],
    );

    let items: Vec<ListItem> = remotes
        .iter()
        .map(|r| {
            let limit = r.limit_upload.as_deref().map(|l| format!("  (limit {l}/s)")).unwrap_or_default();
            ListItem::new(format!("  {}  [{}]{limit}", r.url, r.credentials))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(selection(app.focused_pane == Pane::Remotes));

    let mut state = ListState::default();
    if !remotes.is_empty() {
        state.select(Some(app.selected_remote.min(remotes.len() - 1)));
    }
    f.render_stateful_widget(list, chunks[1], &mut state);
}

fn draw_calls_list(f: &mut Frame, app: &App, area: Rect) {
    let calls = app.current_calls();
    let count = calls.len();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header_style = header(app.focused_pane == Pane::Calls);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("Calls ({count}):"),
            header_style,
        )])),
        chunks[0],
    );

    let items: Vec<ListItem> = calls
        .iter()
        .map(|c| ListItem::new(format!("  {c}")))
        .collect();

    let list = List::new(items)
        .highlight_style(selection(app.focused_pane == Pane::Calls));

    let mut state = ListState::default();
    if !calls.is_empty() {
        state.select(Some(app.selected_call.min(calls.len() - 1)));
    }
    f.render_stateful_widget(list, chunks[1], &mut state);
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let default_hint = match app.focused_pane {
        Pane::Tasks => {
            "[a] add  [d] delete  [e] edit all  [s] set default  [o] open in $EDITOR  [Tab] switch pane  [q] quit"
        }
        Pane::Fields => {
            "[↑↓] select field  [Enter/e] edit field  [Tab] switch pane  [q] quit"
        }
        Pane::Remotes => {
            "[a] add  [d] delete  [e] edit  [t] test  [Tab] switch pane  [q] quit"
        }
        Pane::Calls => {
            "[a] add  [d] delete  [Ctrl+↑↓] reorder  [Tab] switch pane  [q] quit"
        }
    };
    let status = app.status_message.as_deref().unwrap_or(default_hint);
    let para = Paragraph::new(Line::from(vec![Span::raw(format!(" {status}"))]))
        .style(MUTED);
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup_config::BackupConfig;
    use ratatui::{backend::TestBackend, Terminal};

    const KDL: &str = r#"default-task "photos"
tasks {
    task "photos" {
        schedule "weekly sun 03:30"
        before "! pgrep -x Photos"
        backup {
            repo "/r/photos"
            directory "/d"
            password-profile "photos"
            include "/a"
            include "/b"
            steps { sync { every "daily"; window "01:00-06:00" } }
            remote "rustfs:https://h/p" { credentials "rustfs"
limit-upload "20MiB" }
        }
    }
    task "plain" { backup { repo "/r/plain"
directory "/d" } }
}"#;

    /// The screen as text, one string per row.
    fn render(selected_task: usize) -> Vec<String> {
        let mut app = App::new(&BackupConfig::parse("t.kdl", KDL).unwrap(), "t.kdl".to_string());
        app.selected_task = selected_task;
        let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn shows_the_new_settings_for_a_task_that_has_them() {
        let screen = render(0).join("\n");
        for expected in [
            "Schedule",
            "weekly sun 03:30",
            "photos",
            "Also set (press o to edit the file):",
            "Include  /a (+1 more paths)",
            "Before   ! pgrep -x Photos",
            "Steps    sync daily 01:00-06:00",
            "(limit 20MiB/s)",
        ] {
            assert!(screen.contains(expected), "missing {expected:?} in:\n{screen}");
        }
    }

    #[test]
    fn a_plain_task_shows_no_summary_block_and_empty_new_rows() {
        let screen = render(1).join("\n");
        assert!(screen.contains("Schedule") && screen.contains("(none)"));
        assert!(!screen.contains("Also set"), "no extra settings, so no block:\n{screen}");
        assert!(!screen.contains("(limit"));
    }

    #[test]
    fn the_remotes_and_calls_panes_still_fit_below_the_new_rows() {
        let screen = render(0).join("\n");
        assert!(screen.contains("Remotes (1):") && screen.contains("Calls (0):"), "{screen}");
    }
}
