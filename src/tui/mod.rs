mod app;
mod events;
mod ui;

pub use app::App;

use std::io::stdout;
use std::time::Duration;

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

pub fn run(config_path: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("could not read config: {e}"))?;
    let config: crate::backup_config::BackupConfig =
        knuffel::parse(config_path, &content).map_err(|e| e.to_string())?;

    let mut app_state = App::new(&config, config_path.to_string());

    enable_raw_mode().map_err(|e| e.to_string())?;
    execute!(stdout(), EnterAlternateScreen).map_err(|e| e.to_string())?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    let result = run_loop(&mut terminal, &mut app_state);

    disable_raw_mode().ok();
    execute!(stdout(), LeaveAlternateScreen).ok();

    result
}

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<(), String> {
    loop {
        if app.needs_clear {
            terminal.clear().map_err(|e| e.to_string())?;
            app.needs_clear = false;
        }
        terminal.draw(|f| ui::draw(f, app)).map_err(|e| e.to_string())?;

        if event::poll(Duration::from_millis(50)).map_err(|e| e.to_string())? {
            if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
                events::handle_key(app, key);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
