use anyhow::Result;
use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{
    event::{is_quit, Event, EventHandler},
    screens::{ChatListPane, ChatViewPane, OnboardingScreen},
    tui::Tui,
};

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Onboarding,
    Main,
}

#[derive(Debug, Clone, PartialEq)]
enum Focus {
    ContactList,
    ChatView,
    Compose,
}

pub struct App {
    screen: Screen,
    onboarding: OnboardingScreen,
    focus: Focus,
    chat_list: ChatListPane,
    chat_view: ChatViewPane,
    status: String,
    running: bool,
}

impl App {
    pub fn new() -> Self {
        let chat_list = ChatListPane::new();
        let initial_name = chat_list
            .selected_contact()
            .map(|c| c.display_name.clone())
            .unwrap_or_default();
        Self {
            screen: Screen::Onboarding,
            onboarding: OnboardingScreen::new(),
            focus: Focus::ContactList,
            chat_list,
            chat_view: ChatViewPane::new(initial_name),
            status: "Ready".into(),
            running: true,
        }
    }

    pub async fn run(&mut self, terminal: &mut Tui) -> Result<()> {
        let mut events = EventHandler::new();
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            if let Some(event) = events.next().await {
                self.handle_event(event);
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        let Event::Key(key) = event;
        if key.kind != KeyEventKind::Press {
            return;
        }
        match self.screen {
            Screen::Onboarding => self.handle_onboarding(key),
            Screen::Main => self.handle_main(key),
        }
    }

    // ── Onboarding ──────────────────────────────────────────────────────────────

    fn handle_onboarding(&mut self, key: crossterm::event::KeyEvent) {
        use crate::screens::onboarding::OnboardingField;
        match key.code {
            // Allow 'q' to quit only when the username field is empty (nothing typed yet)
            KeyCode::Char('q')
                if key.modifiers == crossterm::event::KeyModifiers::NONE
                    && self.onboarding.focused_field == OnboardingField::Username
                    && self.onboarding.username.is_empty() =>
            {
                self.running = false;
            }
            KeyCode::Char('c') if key.modifiers == crossterm::event::KeyModifiers::CONTROL => {
                self.running = false;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.onboarding.next_field();
            }
            KeyCode::Enter => {
                let (user, pass) = self.onboarding.credentials();
                if user.is_empty() || pass.is_empty() {
                    self.onboarding.status = Some("Username and password are required".into());
                    self.onboarding.is_error = true;
                } else {
                    // TODO: call construct-core auth
                    self.onboarding.status = Some(format!("Connecting as {}…", user));
                    self.onboarding.is_error = false;
                    self.screen = Screen::Main;
                }
            }
            KeyCode::Backspace => {
                self.onboarding.pop_char();
                self.onboarding.status = None;
            }
            KeyCode::Char(c) => {
                self.onboarding.push_char(c);
                self.onboarding.status = None;
            }
            _ => {}
        }
    }

    // ── Main chat UI ────────────────────────────────────────────────────────────

    fn handle_main(&mut self, key: crossterm::event::KeyEvent) {
        if is_quit(&key) && self.focus != Focus::Compose {
            self.running = false;
            return;
        }
        match self.focus {
            Focus::ContactList => match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.chat_list.next(),
                KeyCode::Up | KeyCode::Char('k') => self.chat_list.prev(),
                KeyCode::Enter | KeyCode::Tab => {
                    if let Some(c) = self.chat_list.selected_contact() {
                        self.chat_view.contact_name = c.display_name.clone();
                        self.chat_view.messages.clear();
                    }
                    self.set_focus(Focus::ChatView);
                }
                _ => {}
            },
            Focus::ChatView => match key.code {
                KeyCode::Tab | KeyCode::Char('i') => self.set_focus(Focus::Compose),
                KeyCode::BackTab => self.set_focus(Focus::ContactList),
                KeyCode::Esc => self.set_focus(Focus::ContactList),
                _ => {}
            },
            Focus::Compose => match key.code {
                KeyCode::Esc => self.set_focus(Focus::ChatView),
                KeyCode::Enter => {
                    let text = self.chat_view.take_compose();
                    if !text.trim().is_empty() {
                        use crate::screens::chat_view::{ChatMessage, MessageKind};
                        self.chat_view.messages.push(ChatMessage {
                            id: uuid_placeholder(),
                            kind: MessageKind::Sent,
                            text,
                            time: current_time_hhmm(),
                        });
                        self.status = "Message queued (no backend yet)".into();
                    }
                }
                KeyCode::Backspace => self.chat_view.pop_char(),
                KeyCode::Char(c) => self.chat_view.push_char(c),
                _ => {}
            },
        }
    }

    fn set_focus(&mut self, f: Focus) {
        self.chat_list.focused = f == Focus::ContactList;
        self.chat_view.focused = f == Focus::ChatView;
        self.chat_view.compose_focused = f == Focus::Compose;
        self.focus = f;
    }

    // ── Rendering ───────────────────────────────────────────────────────────────

    fn render(&mut self, frame: &mut Frame) {
        match self.screen {
            Screen::Onboarding => frame.render_widget(&self.onboarding, frame.area()),
            Screen::Main => self.render_main(frame),
        }
    }

    fn render_main(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let root = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

        let title = Paragraph::new(Line::from(vec![
            Span::styled(" ◆ Construct ", Style::default().fg(Color::Cyan)),
            Span::styled("TUI", Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled(
                "Tab=switch  ↑↓/jk=navigate  i=compose  Esc=back  q=quit",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        frame.render_widget(title, root[0]);

        let body = Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)])
            .split(root[1]);
        frame.render_widget(&mut self.chat_list, body[0]);
        frame.render_widget(&mut self.chat_view, body[1]);

        let status = Paragraph::new(Line::from(vec![
            Span::styled(" ● ", Style::default().fg(Color::Green)),
            Span::raw(&self.status),
        ]));
        frame.render_widget(status, root[2]);
    }
}

fn uuid_placeholder() -> String {
    format!(
        "msg-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    )
}

fn current_time_hhmm() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    format!("{:02}:{:02}", h, m)
}
