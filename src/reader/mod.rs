pub mod line_reader;

use std::sync::{mpsc, Arc, Mutex};
use crate::messaging::{ReaderThreadMessage, TerminalThreadMessage};
use crate::reader::line_reader::LineReader;

pub fn reader_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, rx: mpsc::Receiver<ReaderThreadMessage>, term_tx: mpsc::Sender<TerminalThreadMessage>, mut input_reader: Box<dyn LineReader>) {
    let mut line = String::new();

    while let Ok(n) = input_reader.read_line(&mut line) {
        // Note that because read_line is blocking, try_recv might get called late
        // This is fine, as we call exit in the terminal thread bringing everything down with us
        if let Ok(message) = rx.try_recv() {
            match message {
                ReaderThreadMessage::Exit => {
                    break;
                }
            }
        }

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
        {
            let mut lines: std::sync::MutexGuard<'_, &mut Vec<String>> = lines_mtx.lock().expect("Could not take lock in reader_thread");
            lines.push(line.clone());
        }

        line.clear();
        term_tx.send(TerminalThreadMessage::Read).expect("Could not send message to terminal thread");
    }
}