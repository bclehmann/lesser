pub mod line_reader;

use std::sync::{mpsc, Arc, Mutex};
use crate::messaging::{TerminalThreadMessage};
use crate::reader::line_reader::LineReader;
use crate::Source;

pub fn reader_thread_fn(source: Arc<dyn Source>, term_tx: mpsc::Sender<TerminalThreadMessage>) {
    let mut line = String::new();
    let mut reader = source
        .get_reader()
        .expect("Source has no reader")
        .lock()
        .expect("Could not take lock in reader_thread");

    while let Ok(n) = reader.read_line(&mut line) {
        // This isn't really for Windows, it's for Windows terminal emulators running under WSL
        // Since we're in raw mode the emulator won't know what to do with LF-style line endings
        // It's entirely possible that this will break things for normal Unix terminals, so this
        // might need revisiting
        if !line.ends_with("\r\n") {
            if line.ends_with('\n') {
                line.pop();
                line.push_str("\r\n");
            } else if line.ends_with('\r') {
                line.push_str("\n");
            } else {
                line.push_str("\r\n");
            }
        }

        if n == 0 {
            break;
        }
        source.add_line(line.clone()).expect("Could not add new line");

        line.clear();
        term_tx.send(TerminalThreadMessage::Read).expect("Could not send message to terminal thread");
    }
}