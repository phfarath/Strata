use ratatui::style::{Color, Modifier, Style};

pub struct Theme;

impl Theme {
    // Colors
    pub const PRIMARY: Color = Color::Rgb(99, 102, 241);      // Indigo
    pub const SECONDARY: Color = Color::Rgb(14, 165, 233);    // Sky blue
    pub const SUCCESS: Color = Color::Rgb(16, 185, 129);      // Emerald green
    pub const WARNING: Color = Color::Rgb(245, 158, 11);      // Amber
    pub const DANGER: Color = Color::Rgb(244, 63, 94);        // Rose / Coral
    pub const ACCENT: Color = Color::Rgb(168, 85, 247);       // Purple / Violet
    pub const TEXT: Color = Color::Rgb(248, 250, 252);        // Pure bright text
    pub const TEXT_MUTED: Color = Color::Rgb(148, 163, 184);  // Slate 400
    pub const BORDER: Color = Color::Rgb(71, 85, 105);        // Slate 600
    pub const BORDER_FOCUS: Color = Color::Rgb(129, 140, 248);// Indigo light
    pub const CARD_BG: Color = Color::Rgb(30, 41, 59);        // Slate 800

    // Common styles
    pub fn title_style() -> Style {
        Style::default()
            .fg(Self::PRIMARY)
            .add_modifier(Modifier::BOLD)
    }

    pub fn active_tab_style() -> Style {
        Style::default()
            .fg(Self::TEXT)
            .bg(Self::PRIMARY)
            .add_modifier(Modifier::BOLD)
    }

    pub fn inactive_tab_style() -> Style {
        Style::default()
            .fg(Self::TEXT_MUTED)
    }

    pub fn border_style() -> Style {
        Style::default().fg(Self::BORDER)
    }

    pub fn focused_border_style() -> Style {
        Style::default().fg(Self::BORDER_FOCUS)
    }

    pub fn selected_row_style() -> Style {
        Style::default()
            .fg(Self::TEXT)
            .bg(Color::Rgb(51, 65, 85))
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_style(status: &str) -> Style {
        match status.to_lowercase().as_str() {
            "healthy" | "invariant" | "in" | "synced" => {
                Style::default().fg(Self::SUCCESS).add_modifier(Modifier::BOLD)
            }
            "at risk" | "suspicious" | "medium" => {
                Style::default().fg(Self::WARNING).add_modifier(Modifier::BOLD)
            }
            "expired" | "critical" | "high" | "out" | "stale" => {
                Style::default().fg(Self::DANGER).add_modifier(Modifier::BOLD)
            }
            _ => Style::default().fg(Self::TEXT_MUTED),
        }
    }
}
