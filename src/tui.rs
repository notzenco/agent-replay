//! The terminal viewer.
//!
//! A timeline of the session on the left, the detail of the selected event on
//! the right, and a status bar along the bottom.

use std::path::Path;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::diff;
use crate::model::{Event as SessionEvent, EventKind, Session};

/// Open a session in the terminal viewer.
pub fn view(path: &Path) -> Result<()> {
    let session = crate::format::read(path)?;
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal, session);
    ratatui::restore();
    result
}

/// Run the viewer until the user leaves.
fn run(terminal: &mut DefaultTerminal, session: Session) -> Result<()> {
    let mut app = App::new(session);
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.on_key(key);
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// The state of the viewer.
struct App {
    session: Session,
    /// The events that pass the search and the filter, as indexes.
    visible: Vec<usize>,
    /// Where the selection is within `visible`.
    selected: usize,
    list_state: ListState,
    /// The text being searched for.
    query: String,
    /// Whether the user is typing a search.
    searching: bool,
    /// The tool the timeline is restricted to, if any.
    filter: Option<String>,
    /// The tools the session used, in the order the filter cycles through.
    tools: Vec<String>,
    /// How far the detail pane is scrolled.
    detail_scroll: u16,
    should_quit: bool,
}

impl App {
    fn new(session: Session) -> Self {
        let mut tools: Vec<String> = session
            .tool_calls()
            .filter_map(|event| event.tool_name().map(str::to_string))
            .collect();
        tools.sort();
        tools.dedup();
        let mut app = App {
            session,
            visible: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            query: String::new(),
            searching: false,
            filter: None,
            tools,
            detail_scroll: 0,
            should_quit: false,
        };
        app.rebuild();
        app
    }

    /// Work out which events pass the search and the filter.
    fn rebuild(&mut self) {
        let query = self.query.to_lowercase();
        let filter = self.filter.clone();
        self.visible = self
            .session
            .events
            .iter()
            .filter(|event| self.matches(event, &query, filter.as_deref()))
            .map(|event| event.index)
            .collect();
        if self.selected >= self.visible.len() {
            self.selected = self.visible.len().saturating_sub(1);
        }
        self.list_state.select(if self.visible.is_empty() {
            None
        } else {
            Some(self.selected)
        });
    }

    /// Whether an event passes the search and the filter.
    fn matches(&self, event: &SessionEvent, query: &str, filter: Option<&str>) -> bool {
        if let Some(tool) = filter
            && event.tool_name() != Some(tool)
        {
            return false;
        }
        if query.is_empty() {
            return true;
        }
        let haystack = match &event.kind {
            EventKind::ToolCall { name, input } => {
                let mut text = name.clone();
                if let Some(target) = event.target() {
                    text.push(' ');
                    text.push_str(&target);
                }
                if let Ok(value) = serde_json::to_string(input) {
                    text.push(' ');
                    text.push_str(&value);
                }
                text
            }
            other => other.text().unwrap_or_default().to_string(),
        };
        haystack.to_lowercase().contains(query)
    }

    /// The event the selection is on.
    fn selected_event(&self) -> Option<&SessionEvent> {
        self.visible
            .get(self.selected)
            .and_then(|index| self.session.events.get(*index))
    }

    /// Move the selection.
    fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() as isize - 1;
        let next = (self.selected as isize + delta).clamp(0, last);
        self.selected = next as usize;
        self.list_state.select(Some(self.selected));
        self.detail_scroll = 0;
    }

    /// Move to the first or the last event.
    fn move_to_edge(&mut self, last: bool) {
        if self.visible.is_empty() {
            return;
        }
        self.selected = if last { self.visible.len() - 1 } else { 0 };
        self.list_state.select(Some(self.selected));
        self.detail_scroll = 0;
    }

    /// Move to the next or previous tool call.
    fn move_to_tool(&mut self, forward: bool) {
        if self.visible.is_empty() {
            return;
        }
        let start = self.selected as isize;
        let last = self.visible.len() as isize - 1;
        let mut step = 0isize;
        while step <= last {
            let candidate = if forward {
                (start + step + 1).min(last)
            } else {
                (start - step - 1).max(0)
            };
            if candidate == start {
                break;
            }
            let is_tool = self
                .visible
                .get(candidate as usize)
                .and_then(|index| self.session.events.get(*index))
                .and_then(|event| event.tool_name())
                .is_some();
            if is_tool {
                self.selected = candidate as usize;
                self.list_state.select(Some(self.selected));
                self.detail_scroll = 0;
                return;
            }
            step += 1;
        }
    }

    /// Move the filter to the next tool the session used.
    fn cycle_filter(&mut self) {
        let options: Vec<Option<String>> = std::iter::once(None)
            .chain(self.tools.iter().cloned().map(Some))
            .collect();
        let current = self
            .filter
            .clone()
            .and_then(|name| {
                options
                    .iter()
                    .position(|option| option.as_deref() == Some(&name))
            })
            .unwrap_or(0);
        let next = (current + 1) % options.len();
        self.filter = options[next].clone();
        self.selected = 0;
        self.rebuild();
    }

    /// Handle a key press.
    fn on_key(&mut self, key: KeyEvent) {
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                    self.rebuild();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.rebuild();
                }
                KeyCode::Char(c) => {
                    self.query.push(c);
                    self.rebuild();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
            }
            KeyCode::Esc => {
                self.filter = None;
                self.query.clear();
                self.rebuild();
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_to_edge(false),
            KeyCode::Char('G') | KeyCode::End => self.move_to_edge(true),
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('f') => self.cycle_filter(),
            KeyCode::Char('n') => self.move_to_tool(true),
            KeyCode::Char('p') => self.move_to_tool(false),
            KeyCode::Enter => self.detail_scroll = self.detail_scroll.saturating_add(1),
            KeyCode::BackTab => self.detail_scroll = self.detail_scroll.saturating_sub(1),
            KeyCode::PageDown => self.detail_scroll = self.detail_scroll.saturating_add(10),
            KeyCode::PageUp => self.detail_scroll = self.detail_scroll.saturating_sub(10),
            _ => {}
        }
    }

    /// Draw the whole screen.
    fn draw(&self, frame: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(frame.area());

        self.draw_title(frame, chunks[0]);

        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
            .split(chunks[1]);

        self.draw_timeline(frame, panes[0]);
        self.draw_detail(frame, panes[1]);
        self.draw_status(frame, chunks[2]);
    }

    /// The title bar.
    fn draw_title(&self, frame: &mut ratatui::Frame, area: Rect) {
        let usage = self.session.total_usage();
        let line = Line::from(vec![
            Span::styled(
                " agent-replay ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                " {}  {}  {} events  {} tokens in  {} out",
                self.session.id,
                self.session.source.as_str(),
                self.session.events.len(),
                usage.total_input(),
                usage.total_output(),
            )),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }

    /// The timeline of events.
    fn draw_timeline(&self, frame: &mut ratatui::Frame, area: Rect) {
        let title = match &self.filter {
            Some(tool) => format!("Timeline · {tool}"),
            None => "Timeline".to_string(),
        };
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .filter_map(|index| self.session.events.get(*index))
            .map(|event| ListItem::new(timeline_line(event)))
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {title} ")),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Indexed(238))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        frame.render_stateful_widget(list, area, &mut self.list_state.clone());
    }

    /// The detail of the selected event.
    fn draw_detail(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();
        let title = match self.selected_event() {
            Some(event) => match event.kind.name() {
                "tool" => format!("Detail · {}", event.tool_name().unwrap_or("tool")),
                other => format!("Detail · {other}"),
            },
            None => "Detail".to_string(),
        };

        if let Some(event) = self.selected_event() {
            lines = detail_lines(event);
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {title} ")),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.detail_scroll, 0));

        frame.render_widget(paragraph, area);
    }

    /// The status bar.
    fn draw_status(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mut spans = Vec::new();
        if self.searching {
            spans.push(Span::styled("search: ", Style::default().fg(Color::Cyan)));
            spans.push(Span::styled(
                format!("{}{}", self.query, "▏"),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                " /search  f filter  n/p tool  g/G ends  enter scroll  q quit ",
                Style::default().fg(Color::Indexed(245)),
            ));
        }
        spans.push(Span::styled(
            format!(
                " {}/{}",
                if self.visible.is_empty() {
                    0
                } else {
                    self.selected + 1
                },
                self.visible.len()
            ),
            Style::default().fg(Color::Indexed(245)),
        ));
        if !self.filter.is_none() {
            spans.push(Span::styled(
                format!(" filter: {}", self.filter.clone().unwrap_or_default()),
                Style::default().fg(Color::Yellow),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

/// The one-line summary of an event.
fn timeline_line(event: &SessionEvent) -> Line<'static> {
    let (tag, colour) = match event.kind.name() {
        "user" => ("user", Color::Blue),
        "assistant" => ("agent", Color::Magenta),
        "reasoning" => ("think", Color::DarkGray),
        "tool" => ("tool", Color::Yellow),
        "result" => ("out", Color::Green),
        _ => ("note", Color::Gray),
    };
    let mut spans = vec![
        Span::styled(
            format!("{tag:<5}"),
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    match &event.kind {
        EventKind::ToolCall { name, .. } => {
            spans.push(Span::styled(
                name.clone(),
                Style::default().fg(Color::Yellow),
            ));
            if let Some(target) = event.target() {
                spans.push(Span::raw(" "));
                spans.push(Span::raw(shorten(&target, 48)));
            }
        }
        EventKind::ToolResult { is_error, .. } => {
            spans.push(Span::styled(
                if *is_error { "error" } else { "ok" }.to_string(),
                Style::default().fg(if *is_error { Color::Red } else { Color::Green }),
            ));
        }
        other => {
            spans.push(Span::raw(shorten(&other.summary(), 60)));
        }
    }
    Line::from(spans)
}

/// The detail pane of an event.
fn detail_lines(event: &SessionEvent) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    match &event.kind {
        EventKind::ToolCall { name, input } => {
            lines.push(Line::from(vec![
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::raw(event.target().unwrap_or_default()),
            ]));
            lines.push(Line::default());
            if let serde_json::Value::Object(fields) = input {
                for (key, value) in fields {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{key}  "), Style::default().fg(Color::Indexed(245))),
                        Span::raw(value_text(value)),
                    ]));
                }
            }
            if let Some(diff) = diff::diff_for(name, input) {
                lines.push(Line::default());
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("+{} ", diff.added),
                        Style::default().fg(Color::Green),
                    ),
                    Span::styled(
                        format!("−{} ", diff.removed),
                        Style::default().fg(Color::Red),
                    ),
                    Span::styled(
                        diff.path.clone().unwrap_or_default(),
                        Style::default().fg(Color::Indexed(245)),
                    ),
                ]));
                for line in &diff.lines {
                    let style = match line.kind {
                        diff::DiffLineKind::Added => Style::default().fg(Color::Green),
                        diff::DiffLineKind::Removed => Style::default().fg(Color::Red),
                        diff::DiffLineKind::Context => Style::default().fg(Color::DarkGray),
                    };
                    lines.push(Line::from(vec![
                        Span::styled(line.kind.prefix().to_string(), style),
                        Span::styled(line.text.clone(), style),
                    ]));
                }
                if diff.is_truncated() {
                    lines.push(Line::from(Span::styled(
                        format!("… {} of {} lines shown", diff.lines.len(), diff.total_lines),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        }
        EventKind::ToolResult { output, is_error } => {
            let style = if *is_error {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            for line in output.lines() {
                lines.push(Line::from(Span::styled(line.to_string(), style)));
            }
        }
        other => {
            for line in other.text().unwrap_or_default().lines() {
                lines.push(Line::from(line.to_string()));
            }
        }
    }
    if let Some(usage) = event.usage {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!(
                "in {}  out {}  cache read {}  reasoning {}",
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.reasoning_tokens
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

/// The text of a value from a tool's input.
fn value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// Shorten a string so it fits on a line.
fn shorten(text: &str, width: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= width {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn app() -> App {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("claude-session.jsonl");
        App::new(crate::format::read(&path).unwrap())
    }

    #[test]
    fn starts_with_every_event_shown() {
        let app = app();
        assert_eq!(app.visible.len(), app.session.events.len());
    }

    #[test]
    fn a_search_hides_the_events_that_do_not_match() {
        let mut app = app();
        app.query = "health".to_string();
        app.rebuild();
        assert!(!app.visible.is_empty());
        assert!(app.visible.len() < app.session.events.len());
    }

    #[test]
    fn a_search_matches_a_tool_name() {
        let mut app = app();
        app.query = "bash".to_string();
        app.rebuild();
        assert_eq!(app.visible.len(), 1);
    }

    #[test]
    fn a_search_looks_inside_the_input_of_a_tool_call() {
        let mut app = app();
        app.query = "run the test suite".to_string();
        app.rebuild();
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.selected_event().unwrap().tool_name(), Some("Bash"));
    }

    #[test]
    fn clearing_the_search_shows_everything_again() {
        let mut app = app();
        app.query = "health".to_string();
        app.rebuild();
        let fewer = app.visible.len();
        app.query.clear();
        app.rebuild();
        assert!(app.visible.len() > fewer);
    }

    #[test]
    fn a_filter_shows_only_one_tool() {
        let mut app = app();
        // The tools are listed in a fixed order, so the cycle is predictable.
        assert_eq!(app.tools, vec!["Bash", "Edit", "Read"]);
        app.cycle_filter();
        assert_eq!(app.filter.as_deref(), Some("Bash"));
        app.cycle_filter();
        assert_eq!(app.filter.as_deref(), Some("Edit"));
        assert_eq!(app.visible.len(), 1);
    }

    #[test]
    fn the_filter_cycles_back_to_everything() {
        let mut app = app();
        for _ in 0..app.tools.len() + 1 {
            app.cycle_filter();
        }
        assert_eq!(app.filter, None);
    }

    #[test]
    fn moving_stays_inside_the_list() {
        let mut app = app();
        app.move_by(-5);
        assert_eq!(app.selected, 0);
        app.move_by(1000);
        assert_eq!(app.selected, app.visible.len() - 1);
    }

    #[test]
    fn moving_to_a_tool_call_skips_the_events_that_are_not_tool_calls() {
        let mut app = app();
        app.move_to_edge(false);
        app.move_to_tool(true);
        let event = app.selected_event().unwrap();
        assert_eq!(event.tool_name(), Some("Read"));
    }

    #[test]
    fn moving_back_to_a_tool_call_finds_the_previous_one() {
        let mut app = app();
        app.move_to_edge(true);
        app.move_to_tool(false);
        let event = app.selected_event().unwrap();
        assert_eq!(event.tool_name(), Some("Bash"));
    }

    #[test]
    fn a_failed_tool_result_is_marked_as_an_error() {
        let app = app();
        let errors = app
            .session
            .events
            .iter()
            .filter(|event| matches!(&event.kind, EventKind::ToolResult { is_error: true, .. }))
            .count();
        assert_eq!(errors, 1);
    }
}
