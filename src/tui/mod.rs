mod app;
pub(crate) mod credentials;
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
    let config = crate::backup_config::BackupConfig::from_file(config_path)?;

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
        app.tick_status();
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
