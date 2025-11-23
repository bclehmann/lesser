
pub enum TerminalThreadMessage {
    KeyEvent(crossterm::event::KeyEvent),
    Resize(u16, u16),
    Read
}
