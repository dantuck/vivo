use colored::*;
use term_size;

pub fn print_banner() {
    #[rustfmt::skip]
    let glyphs: &[(&[[u8; 5]; 7], bool)] = &[
        (&[[1,0,0,0,1],[1,0,0,0,1],[1,0,0,0,1],[0,1,0,1,0],[0,1,0,1,0],[0,0,1,0,0],[0,0,1,0,0]], false), // V
        (&[[1,1,1,1,1],[0,0,1,0,0],[0,0,1,0,0],[0,0,1,0,0],[0,0,1,0,0],[0,0,1,0,0],[1,1,1,1,1]], false), // I
        (&[[1,0,0,0,1],[1,0,0,0,1],[1,0,0,0,1],[0,1,0,1,0],[0,1,0,1,0],[0,0,1,0,0],[0,0,1,0,0]], false), // V
        (&[[0,1,1,1,0],[1,0,0,0,1],[1,0,0,0,1],[1,0,0,0,1],[1,0,0,0,1],[1,0,0,0,1],[0,1,1,1,0]], false), // O
        (&[[1,0,0,0,1],[1,0,0,1,0],[1,0,1,0,0],[1,1,0,0,0],[1,0,1,0,0],[1,0,0,1,0],[1,0,0,0,1]], true),  // K
        (&[[1,1,1,1,1],[1,0,0,0,0],[1,0,0,0,0],[1,1,1,1,0],[1,0,0,0,0],[1,0,0,0,0],[1,1,1,1,1]], true),  // E
        (&[[1,1,1,1,1],[1,0,0,0,0],[1,0,0,0,0],[1,1,1,1,0],[1,0,0,0,0],[1,0,0,0,0],[1,1,1,1,1]], true),  // E
        (&[[1,1,1,1,0],[1,0,0,0,1],[1,0,0,0,1],[1,1,1,1,0],[1,0,0,0,0],[1,0,0,0,0],[1,0,0,0,0]], true),  // P
    ];

    const DIM:   (u8, u8, u8) = (42, 53, 85);
    const WHITE: (u8, u8, u8) = (241, 245, 249);
    const GREEN: (u8, u8, u8) = (52, 211, 153);

    // Render 7 glyph rows as 4 terminal lines using half-block characters.
    // Each line covers two glyph rows: ▀ = top lit, ▄ = bot lit, █ = both lit.
    println!();
    for pair in 0..4_usize {
        let r_top = pair * 2;
        let r_bot = r_top + 1;
        let mut line = String::new();

        for (gi, (rows, green)) in glyphs.iter().enumerate() {
            let (lr, lg, lb) = if *green { GREEN } else { WHITE };
            for col in 0..5_usize {
                let top = r_top < 7 && rows[r_top][col] == 1;
                let bot = r_bot < 7 && rows[r_bot][col] == 1;
                let s = match (top, bot) {
                    (true,  true)  => "█".truecolor(lr, lg, lb),
                    (true,  false) => "▀".truecolor(lr, lg, lb).on_truecolor(DIM.0, DIM.1, DIM.2),
                    (false, true)  => "▄".truecolor(lr, lg, lb).on_truecolor(DIM.0, DIM.1, DIM.2),
                    (false, false) => "·".truecolor(DIM.0, DIM.1, DIM.2),
                };
                line.push_str(&s.to_string());
            }
            // interpunct separator: same width gap as between letters, dot lit on glyph row 3
            if gi == 3 {
                line.push(' ');
                let s = if r_bot == 3 {
                    "▄".truecolor(GREEN.0, GREEN.1, GREEN.2).on_truecolor(DIM.0, DIM.1, DIM.2)
                } else {
                    "·".truecolor(DIM.0, DIM.1, DIM.2)
                };
                line.push_str(&s.to_string());
                line.push(' ');
            } else if gi < glyphs.len() - 1 {
                line.push(' ');
            }
        }
        println!("{}", line);
    }
    println!();
}

pub fn section_header(message: &str) {
    let prefix_dashes = "----";
    let max_line_length = 70;

    let term_width = term_size::dimensions()
        .map(|(w, _)| w)
        .unwrap_or(max_line_length);
    let line_length = term_width.min(max_line_length);

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
