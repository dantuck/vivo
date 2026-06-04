use crossterm::event::{KeyCode, KeyEvent};
use super::app::App;

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
        app.should_quit = true;
    }
}
