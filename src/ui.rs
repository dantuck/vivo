use colored::*;
use term_size;

pub fn section_header(message: &str) {
    // Define the base number of dashes and max line length
    let prefix_dashes = "----"; // Start with 4 dashes
    let max_line_length = 70; // Maximum allowed line length

    // Get terminal width or use 50 as a fallback
    let term_width = term_size::dimensions()
        .map(|(w, _)| w)
        .unwrap_or(max_line_length);
    let line_length = term_width.min(max_line_length); // Don't exceed 70 chars

    // Calculate the number of dashes needed to fill the remaining space
    let message_len = message.len();
    let dashes_needed = line_length.saturating_sub(prefix_dashes.len() + message_len + 2);

    let task_message = format!(
        "{} {} {}",
        prefix_dashes.blue(),
        message.green(),
        "-".repeat(dashes_needed).blue()
    );

    println!();
    println!("{}", task_message);
}

pub fn info(message: &str) {
    println!("[{}] {}", "i".blue(), message);
}
