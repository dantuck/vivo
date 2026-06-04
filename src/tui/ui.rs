use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use super::app::{App, Pane};

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
    draw_remotes(f, app, panes[1]);
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

fn draw_remotes(f: &mut Frame, app: &App, area: Rect) {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.as_str())
        .unwrap_or("—");
    let title = format!(" Task: {task_name} ");

    let remotes = app.current_remotes();
    let items: Vec<ListItem> = remotes
        .iter()
        .map(|r| ListItem::new(format!("{}  [{}]", r.url, r.credentials)))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(focused_border(app.focused_pane == Pane::Remotes)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !remotes.is_empty() {
        state.select(Some(app.selected_remote));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let status = app
        .status_message
        .as_deref()
        .unwrap_or("[a] add  [d] delete  [e] edit in $EDITOR  [Tab] switch pane  [q] quit");
    let para = Paragraph::new(Line::from(vec![Span::raw(format!(" {status}"))]))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(para, area);
}
