use std::sync::mpsc;
use std::time::Duration;
use crossterm::event::{poll, read, Event};
use crate::messaging::{InputThreadMessage, TerminalThreadMessage};

pub fn input_thread_fn(term_tx: mpsc::Sender<TerminalThreadMessage>, input_rx: mpsc::Receiver<InputThreadMessage>) {
    loop {
        if let Ok(message) = input_rx.try_recv() {
            match message {
                InputThreadMessage::Exit => {
                    break;
                }
            }
        }
        match poll(Duration::from_millis(100)).unwrap() {
            true => {
                match read().unwrap() {
                    Event::Key(event) => {
                        match term_tx.send(TerminalThreadMessage::KeyEvent(event)) {
                            Err(_) => {
                                break;
                            }
                            _ => {}
                        }
                    },
                    Event::Resize(cols, rows) => {
                        match term_tx.send(TerminalThreadMessage::Resize(cols, rows)) {
                            Err(_) => {
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ => {
                    }
                }
            },
            false => {
                continue;
            }
        }
    }
}
