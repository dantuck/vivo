use ratatui::Frame;
use super::app::App;

pub fn draw(f: &mut Frame, _app: &App) {
    use ratatui::widgets::{Block, Borders};
    let block = Block::default().borders(Borders::ALL).title("vivo manage");
    f.render_widget(block, f.area());
}
