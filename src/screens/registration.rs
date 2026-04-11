use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

pub const STEPS: &[&str] = &[
    "Generating signing key  (Ed25519)",
    "Generating identity key (X25519)",
    "Generating signed pre-key (X25519)",
    "Signing pre-key",
    "Connecting to Construct",
    "Solving proof-of-work",
    "Registering identity",
];

/// "Chaos → Order" animation: scattered dots gradually converge into a solid line.
/// Each frame is exactly 3 display columns (placed inside [ ] brackets in the render).
const SPINNER: &[&str] = &[
    "∙ ∙", // scattered
    " ∙ ", // centre
    "∙∙ ", // drifting left
    " ∙∙", // drifting right
    "·∙·", // three uneven
    "·→·", // converging inward
    "·──", // line forming
    "───", // ORDER achieved
];

pub struct RegistrationScreen {
    /// How many steps have been *started* (index of the currently active step).
    /// `active_step == STEPS.len()` means all steps are complete.
    pub active_step: usize,
    /// Incremented by periodic Tick events for spinner animation.
    pub spinner_tick: u8,
}

impl RegistrationScreen {
    pub fn new() -> Self {
        Self {
            active_step: 0,
            spinner_tick: 0,
        }
    }

    pub fn advance(&mut self, step: usize) {
        if step > self.active_step {
            self.active_step = step;
        }
    }

    pub fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
    }
}

impl Widget for &RegistrationScreen {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Height: title(1) + gap(1) + steps(N) + gap(1) + hint(1)
        let n = STEPS.len() as u16;
        let content_h = 1 + 1 + n + 1 + 1;
        let v_offset = area.height.saturating_sub(content_h) / 2;
        let mut y = area.y + v_offset;

        // ── Title ─────────────────────────────────────────────────────────────
        let title = "INITIALIZING IDENTITY";
        let tw = title.chars().count() as u16;
        let tx = area.x + area.width.saturating_sub(tw) / 2;
        Paragraph::new(title)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .render(
                Rect {
                    x: tx,
                    y,
                    width: tw.min(area.width),
                    height: 1,
                },
                buf,
            );
        y += 2;

        // ── Step list ─────────────────────────────────────────────────────────
        // All prefixes are 5 display cols: [xxx] where xxx is 3 chars.
        let label_max = STEPS.iter().map(|s| s.chars().count()).max().unwrap_or(0) as u16;
        let row_w = 7 + label_max; // "[xxx] " (6) + 1 space margin
        let row_x = area.x + area.width.saturating_sub(row_w) / 2;

        for (i, label) in STEPS.iter().enumerate() {
            let frame = SPINNER[(self.spinner_tick as usize) % SPINNER.len()];
            let (prefix, prefix_color, label_color): (String, Color, Color) =
                if i < self.active_step {
                    ("[ ✓ ]".into(), Color::Green, Color::DarkGray)
                } else if i == self.active_step {
                    (format!("[{frame}]"), Color::Cyan, Color::White)
                } else {
                    ("[   ]".into(), Color::DarkGray, Color::DarkGray)
                };

            let line = Line::from(vec![
                Span::styled(
                    format!("{prefix} "),
                    Style::default()
                        .fg(prefix_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(*label, Style::default().fg(label_color)),
            ]);

            Paragraph::new(line).render(
                Rect {
                    x: row_x,
                    y,
                    width: row_w.min(area.width.saturating_sub(row_x.saturating_sub(area.x))),
                    height: 1,
                },
                buf,
            );
            y += 1;
        }
        y += 1;

        // ── Hint ──────────────────────────────────────────────────────────────
        let hint = "Ctrl+C to abort";
        let hw = hint.len() as u16;
        let hx = area.x + area.width.saturating_sub(hw) / 2;
        Paragraph::new(hint)
            .style(Style::default().fg(Color::DarkGray))
            .render(
                Rect {
                    x: hx,
                    y,
                    width: hw.min(area.width),
                    height: 1,
                },
                buf,
            );
    }
}
