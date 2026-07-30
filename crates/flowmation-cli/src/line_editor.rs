use std::io::{self, IsTerminal, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;

const DIM: &str = "\u{1b}[2m";
const RESET: &str = "\u{1b}[0m";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    Text(String),
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    Backspace,
    Delete,
    Tab,
    Enter,
    CtrlC,
    CtrlD,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorAction {
    Continue,
    Submit(String),
    Exit,
    InterruptForeground,
}

#[derive(Debug)]
pub struct LineEditor {
    buffer: String,
    cursor: usize,
    exit_armed: bool,
    history: Vec<String>,
    history_index: usize,
    draft: String,
    commands: Vec<String>,
}

pub fn read_line(
    prompt: &str,
    history: Vec<String>,
    commands: Vec<String>,
    foreground_active: bool,
) -> Result<Option<String>, String> {
    if !io::stdin().is_terminal() {
        print!("{prompt}");
        io::stdout()
            .flush()
            .map_err(|error| format!("could not write prompt: {error}"))?;
        let mut input = String::new();
        let count = io::stdin()
            .read_line(&mut input)
            .map_err(|error| format!("could not read input: {error}"))?;
        return Ok((count > 0).then_some(input.trim_end_matches(['\r', '\n']).to_owned()));
    }
    read_tty_line(prompt, history, commands, foreground_active)
}

fn read_tty_line(
    prompt: &str,
    history: Vec<String>,
    commands: Vec<String>,
    foreground_active: bool,
) -> Result<Option<String>, String> {
    let raw_mode = RawModeGuard::enable()
        .map_err(|error| format!("could not enable terminal input mode: {error}"))?;

    let result = (|| {
        let mut output = io::stdout().lock();
        let mut columns = terminal_columns();
        let mut editor = LineEditor::new(history, commands);
        let mut renderer = TerminalRenderer::default();
        output
            .write_all(
                renderer
                    .render(
                        prompt,
                        editor.buffer(),
                        editor.completion().unwrap_or_default(),
                        0,
                        columns,
                    )
                    .as_bytes(),
            )
            .and_then(|()| output.flush())
            .map_err(|error| format!("could not render prompt: {error}"))?;
        loop {
            let terminal_event =
                event::read().map_err(|error| format!("could not read terminal input: {error}"))?;
            let key = match terminal_event {
                Event::Key(key_event) => {
                    let Some(key) = key_from_event(key_event) else {
                        continue;
                    };
                    key
                }
                Event::Paste(text) => Key::Text(text),
                Event::Resize(width, _) => {
                    columns = usize::from(width).max(1);
                    output
                        .write_all(
                            renderer
                                .render(
                                    prompt,
                                    editor.buffer(),
                                    editor.completion().unwrap_or_default(),
                                    editor.cursor,
                                    columns,
                                )
                                .as_bytes(),
                        )
                        .and_then(|()| output.flush())
                        .map_err(|error| format!("could not render prompt: {error}"))?;
                    continue;
                }
                Event::FocusGained | Event::FocusLost | Event::Mouse(_) => continue,
            };
            match editor.handle(key, foreground_active) {
                EditorAction::Continue => {
                    output
                        .write_all(
                            renderer
                                .render(
                                    prompt,
                                    editor.buffer(),
                                    editor.completion().unwrap_or_default(),
                                    editor.cursor,
                                    columns,
                                )
                                .as_bytes(),
                        )
                        .and_then(|()| output.flush())
                        .map_err(|error| format!("could not render prompt: {error}"))?;
                }
                EditorAction::Submit(value) => {
                    output
                        .write_all(renderer.finish().as_bytes())
                        .and_then(|()| output.flush())
                        .map_err(|error| format!("could not render prompt: {error}"))?;
                    return Ok(Some(value));
                }
                EditorAction::Exit | EditorAction::InterruptForeground => {
                    output
                        .write_all(renderer.finish().as_bytes())
                        .and_then(|()| output.flush())
                        .map_err(|error| format!("could not render prompt: {error}"))?;
                    return Ok(None);
                }
            }
        }
    })();
    let restore = raw_mode
        .restore()
        .map_err(|error| format!("could not restore terminal input mode: {error}"));
    match result {
        Ok(value) => restore.map(|()| value),
        Err(error) => Err(error),
    }
}

struct RawModeGuard {
    enabled: bool,
}

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self { enabled: true })
    }

    fn restore(mut self) -> io::Result<()> {
        let result = terminal::disable_raw_mode();
        self.enabled = false;
        result
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.enabled {
            let _result = terminal::disable_raw_mode();
        }
    }
}

fn terminal_columns() -> usize {
    terminal::size()
        .map_or(80, |(width, _)| usize::from(width))
        .max(1)
}

fn key_from_event(event: KeyEvent) -> Option<Key> {
    if event.kind == KeyEventKind::Release {
        return None;
    }
    match event.code {
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => Some(Key::CtrlC),
        KeyCode::Char('d') if event.modifiers.contains(KeyModifiers::CONTROL) => Some(Key::CtrlD),
        KeyCode::Char(value) if !event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Key::Text(value.to_string()))
        }
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::Enter => Some(Key::Enter),
        _ => None,
    }
}

impl LineEditor {
    #[must_use]
    pub fn new(history: Vec<String>, mut commands: Vec<String>) -> Self {
        commands.sort();
        commands.dedup();
        let history_index = history.len();
        Self {
            buffer: String::new(),
            cursor: 0,
            exit_armed: false,
            history,
            history_index,
            draft: String::new(),
            commands,
        }
    }

    #[must_use]
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    #[must_use]
    pub fn completion(&self) -> Option<&str> {
        if self.cursor != self.buffer.len() {
            return None;
        }
        let typed = self.buffer.strip_prefix('/')?;
        if typed.is_empty() || typed.chars().any(char::is_whitespace) {
            return None;
        }
        self.commands
            .iter()
            .find(|command| command.starts_with(typed) && command.as_str() != typed)
            .map(|command| &command[typed.len()..])
    }

    pub fn handle(&mut self, key: Key, foreground_active: bool) -> EditorAction {
        if key == Key::CtrlC {
            if foreground_active {
                return EditorAction::InterruptForeground;
            }
            if !self.buffer.is_empty() {
                self.buffer.clear();
                self.cursor = 0;
                self.exit_armed = false;
                self.reset_navigation();
                return EditorAction::Continue;
            }
            if self.exit_armed {
                return EditorAction::Exit;
            }
            self.exit_armed = true;
            return EditorAction::Continue;
        }
        self.exit_armed = false;
        match key {
            Key::Text(text) => {
                self.buffer.insert_str(self.cursor, &text);
                self.cursor += text.len();
            }
            Key::Left => self.cursor = previous_boundary(&self.buffer, self.cursor),
            Key::Right if self.cursor == self.buffer.len() => {
                self.accept_completion();
            }
            Key::Right => self.cursor = next_boundary(&self.buffer, self.cursor),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = self.buffer.len(),
            Key::Up => self.previous_history(),
            Key::Down => self.next_history(),
            Key::Backspace => {
                let previous = previous_boundary(&self.buffer, self.cursor);
                self.buffer.replace_range(previous..self.cursor, "");
                self.cursor = previous;
            }
            Key::Delete => {
                let next = next_boundary(&self.buffer, self.cursor);
                self.buffer.replace_range(self.cursor..next, "");
            }
            Key::Enter => return EditorAction::Submit(self.buffer.clone()),
            Key::CtrlD if self.buffer.is_empty() => return EditorAction::Exit,
            Key::Tab => {
                self.accept_completion();
            }
            Key::CtrlC | Key::CtrlD => {}
        }
        EditorAction::Continue
    }

    fn accept_completion(&mut self) -> bool {
        let Some(completion) = self.completion().map(str::to_owned) else {
            return false;
        };
        self.buffer.push_str(&completion);
        self.cursor = self.buffer.len();
        true
    }

    fn previous_history(&mut self) {
        if self.history_index == 0 {
            return;
        }
        if self.history_index == self.history.len() {
            self.draft.clone_from(&self.buffer);
        }
        self.history_index -= 1;
        self.buffer.clone_from(&self.history[self.history_index]);
        self.cursor = self.buffer.len();
    }

    fn next_history(&mut self) {
        if self.history_index == self.history.len() {
            return;
        }
        self.history_index += 1;
        if self.history_index == self.history.len() {
            self.buffer.clone_from(&self.draft);
        } else {
            self.buffer.clone_from(&self.history[self.history_index]);
        }
        self.cursor = self.buffer.len();
    }

    fn reset_navigation(&mut self) {
        self.history_index = self.history.len();
        self.draft.clear();
    }
}

#[derive(Debug, Default)]
struct TerminalRenderer {
    rendered_cursor_row: usize,
    rendered_end_row: usize,
}

impl TerminalRenderer {
    fn render(
        &mut self,
        prompt: &str,
        buffer: &str,
        completion: &str,
        cursor: usize,
        columns: usize,
    ) -> String {
        let prompt_width = prompt.chars().count();
        let end_offset = prompt_width + buffer.chars().count() + completion.chars().count();
        let cursor_width = buffer[..cursor].chars().count();
        let target_offset = prompt_width + cursor_width;
        let end_row = end_offset / columns;
        let end_column = end_offset % columns;
        let target_row = target_offset / columns;
        let target_column = target_offset % columns;
        let mut rendered = String::from("\r");
        if self.rendered_cursor_row > 0 {
            rendered.push_str(&format!("\u{1b}[{}A", self.rendered_cursor_row));
        }
        rendered.push_str("\u{1b}[0J");
        rendered.push_str(prompt);
        rendered.push_str(buffer);
        if !completion.is_empty() {
            rendered.push_str(DIM);
            rendered.push_str(completion);
            rendered.push_str(RESET);
        }
        if end_offset > 0 && end_column == 0 {
            rendered.push(' ');
        }
        if end_row > target_row {
            rendered.push_str(&format!("\u{1b}[{}A", end_row - target_row));
        }
        rendered.push_str(&format!("\u{1b}[{}G", target_column + 1));
        self.rendered_cursor_row = target_row;
        self.rendered_end_row = end_row;
        rendered
    }

    fn finish(&self) -> String {
        let rows_below_cursor = self
            .rendered_end_row
            .saturating_sub(self.rendered_cursor_row);
        if rows_below_cursor == 0 {
            "\r\n".to_owned()
        } else {
            format!("\u{1b}[{rows_below_cursor}B\r\n")
        }
    }
}

fn previous_boundary(value: &str, index: usize) -> usize {
    value[..index]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(value: &str, index: usize) -> usize {
    value[index..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(offset, _)| index + offset)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::{EditorAction, Key, LineEditor, TerminalRenderer, key_from_event};

    #[test]
    fn maps_cross_platform_terminal_events() {
        assert_eq!(
            key_from_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            Some(Key::Left)
        );
        assert_eq!(
            key_from_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Key::CtrlC)
        );
        assert_eq!(
            key_from_event(KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE)),
            Some(Key::Text("é".to_owned()))
        );
    }

    #[test]
    fn ignores_release_and_unhandled_control_events() {
        assert_eq!(
            key_from_event(KeyEvent::new_with_kind(
                KeyCode::Enter,
                KeyModifiers::NONE,
                KeyEventKind::Release
            )),
            None
        );
        assert_eq!(
            key_from_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn browses_history_and_restores_draft() {
        let mut editor = LineEditor::new(vec!["first".to_owned(), "second".to_owned()], Vec::new());
        editor.handle(Key::Text("draft".to_owned()), false);
        editor.handle(Key::Up, false);
        assert_eq!(editor.buffer(), "second");
        editor.handle(Key::Up, false);
        assert_eq!(editor.buffer(), "first");
        editor.handle(Key::Down, false);
        editor.handle(Key::Down, false);
        assert_eq!(editor.buffer(), "draft");
    }

    #[test]
    fn clears_text_then_requires_two_interrupts_to_exit() {
        let mut editor = LineEditor::new(Vec::new(), Vec::new());
        editor.handle(Key::Text("draft".to_owned()), false);
        assert_eq!(editor.handle(Key::CtrlC, false), EditorAction::Continue);
        assert!(editor.buffer().is_empty());
        assert_eq!(editor.handle(Key::CtrlC, false), EditorAction::Continue);
        assert_eq!(editor.handle(Key::CtrlC, false), EditorAction::Exit);
    }

    #[test]
    fn delegates_interrupt_to_foreground_operation() {
        let mut editor = LineEditor::new(Vec::new(), Vec::new());
        assert_eq!(
            editor.handle(Key::CtrlC, true),
            EditorAction::InterruptForeground
        );
    }

    #[test]
    fn supports_cursor_and_deletion_keys() {
        let mut editor = LineEditor::new(Vec::new(), Vec::new());
        editor.handle(Key::Text("ab".to_owned()), false);
        editor.handle(Key::Left, false);
        editor.handle(Key::Right, false);
        editor.handle(Key::Home, false);
        editor.handle(Key::Delete, false);
        editor.handle(Key::End, false);
        editor.handle(Key::Backspace, false);
        editor.handle(Key::Tab, false);
        assert_eq!(
            editor.handle(Key::Enter, false),
            EditorAction::Submit(String::new())
        );
        assert_eq!(
            LineEditor::new(Vec::new(), Vec::new()).handle(Key::CtrlD, false),
            EditorAction::Exit
        );
    }

    #[test]
    fn offers_and_accepts_sorted_command_completions() {
        let mut editor = LineEditor::new(
            Vec::new(),
            vec!["workflows".to_owned(), "workflow".to_owned()],
        );
        editor.handle(Key::Text("/work".to_owned()), false);
        assert_eq!(editor.completion(), Some("flow"));

        editor.handle(Key::Tab, false);
        assert_eq!(editor.buffer(), "/workflow");
        assert_eq!(editor.completion(), Some("s"));

        editor.handle(Key::Right, false);
        assert_eq!(editor.buffer(), "/workflows");
        assert_eq!(editor.completion(), None);
    }

    #[test]
    fn only_completes_a_command_at_the_end_of_the_first_token() {
        let mut editor = LineEditor::new(Vec::new(), vec!["workflow".to_owned()]);
        editor.handle(Key::Text("/work".to_owned()), false);
        editor.handle(Key::Left, false);
        assert_eq!(editor.completion(), None);
        editor.handle(Key::End, false);
        editor.handle(Key::Text(" input".to_owned()), false);
        assert_eq!(editor.completion(), None);

        let mut message = LineEditor::new(Vec::new(), vec!["workflow".to_owned()]);
        message.handle(Key::Text("work".to_owned()), false);
        assert_eq!(message.completion(), None);
    }

    #[test]
    fn clears_every_previously_rendered_row_when_input_wraps() {
        let mut renderer = TerminalRenderer::default();
        let first = renderer.render("> ", "123456789", "", 9, 10);
        assert!(first.contains("> 123456789"));
        let second = renderer.render("> ", "1234567890", "", 10, 10);
        assert!(second.contains("\u{1b}[1A\u{1b}[0J> 1234567890"));
    }

    #[test]
    fn renders_completion_dimmed_without_moving_the_input_cursor() {
        let mut renderer = TerminalRenderer::default();
        let rendered = renderer.render("> ", "/work", "flow", 5, 80);
        assert!(rendered.contains("> /work\u{1b}[2mflow\u{1b}[0m"));
        assert!(rendered.ends_with("\u{1b}[8G"));
    }

    #[test]
    fn finishes_raw_prompt_at_the_first_column_after_all_input_rows() {
        let mut renderer = TerminalRenderer::default();
        renderer.render("> ", "1234567890", "", 0, 10);
        assert_eq!(renderer.finish(), "\u{1b}[1B\r\n");

        renderer.render("> ", "answer", "", 6, 80);
        assert_eq!(renderer.finish(), "\r\n");
    }
}
