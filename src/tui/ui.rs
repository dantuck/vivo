use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use super::app::{App, Pane};

// Pre-padded to 14 chars — avoids a format! allocation per field per frame.
const FIELD_LABELS: [&str; 6] = [
    "Name          ",
    "Description   ",
    "Repo          ",
    "Directory     ",
    "Exclude file  ",
    "Files from    ",
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
    if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn draw_tasks(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.tasks.iter().map(|t| ListItem::new(t.name.as_str())).collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Tasks ")
                .border_style(focused_border(app.focused_pane == Pane::Tasks)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Min(0),
        ])
        .split(inner);

    draw_task_fields(f, app, chunks[0]);
    draw_remotes_list(f, app, chunks[1]);
    draw_calls_list(f, app, chunks[2]);
}

fn draw_task_fields(f: &mut Frame, app: &App, area: Rect) {
    let task = match app.tasks.get(app.selected_task) {
        Some(t) => t,
        None => return,
    };

    let in_fields = app.focused_pane == Pane::Fields;
    let label_style = Style::default().fg(Color::DarkGray);
    let dim = label_style.add_modifier(Modifier::DIM);
    let highlight = Style::default().add_modifier(Modifier::REVERSED);

    let values: [Option<&str>; 6] = [
        Some(task.name.as_str()),
        task.description.as_deref(),
        task.repo.as_deref(),
        task.directory.as_deref(),
        task.exclude_file.as_deref(),
        task.files_from.as_deref(),
    ];

    let lines: Vec<Line> = FIELD_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let selected = in_fields && i == app.selected_field;
            let value_span = match values[i] {
                Some(v) if !v.is_empty() => Span::raw(v.to_string()),
                _ => Span::styled("(none)", dim),
            };
            let line = Line::from(vec![Span::styled(*label, label_style), value_span]);
            if selected { line.style(highlight) } else { line }
        })
        .collect();

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_remotes_list(f: &mut Frame, app: &App, area: Rect) {
    let remotes = app.current_remotes();
    let count = remotes.len();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header_style = if app.focused_pane == Pane::Remotes {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("Remotes ({count}):"),
            header_style,
        )])),
        chunks[0],
    );

    let items: Vec<ListItem> = remotes
        .iter()
        .map(|r| ListItem::new(format!("  {}  [{}]", r.url, r.credentials)))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !remotes.is_empty() && app.focused_pane == Pane::Remotes {
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

    let header_style = if app.focused_pane == Pane::Calls {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
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
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !calls.is_empty() && app.focused_pane == Pane::Calls {
        state.select(Some(app.selected_call.min(calls.len() - 1)));
    }
    f.render_stateful_widget(list, chunks[1], &mut state);
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let default_hint = match app.focused_pane {
        Pane::Tasks => {
            "[a] add  [d] delete  [e] edit all  [o] open in $EDITOR  [Tab] switch pane  [q] quit"
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
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(para, area);
}
