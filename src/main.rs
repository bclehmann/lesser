mod terminal;
mod messaging;
mod input;
mod reader;

use std::{fs::File, io::{BufRead, Write}, sync::{mpsc, Arc, Mutex}, thread};
use std::io::{Read, Seek};
use clap::Parser;
use notify::{Watcher};
use crate::input::input_thread_fn;
use crate::messaging::{InputThreadMessage, ReaderThreadMessage, TerminalThreadMessage};
use crate::reader::line_reader::{FileReader, LineReader, StdinReader, WatchingFileReader};
use crate::reader::reader_thread_fn;
use crate::terminal::term_thread_fn;

#[cfg(unix)]
fn get_tty() -> File {
    File::open("/dev/tty").expect("Could not open /dev/tty")
}

#[cfg(windows)]
fn get_tty() -> File {
    // CON is the equivalent to /dev/tty on Windows
    File::open("CON").expect("Could not open CON")
}

#[derive(clap::Parser)]
#[derive(Debug)]
struct Args {
    filename: Option<String>,

    #[arg(long)]
    watch: bool,
}

fn main() {
    let args = Args::parse();

    let input_reader: Box<dyn LineReader> = match &args.filename {
        Some(filename) => {
            if !args.watch {
                Box::new(FileReader::new(filename))
            } else {
                Box::new(WatchingFileReader::new(filename))
            }
        }
        None => {
            Box::new(StdinReader::new())
        }
    };

    let mut lines_raw = Vec::<String>::new();
    let lines_mtx = Arc::new(Mutex::new(&mut lines_raw));
    let reader_thread_mtx = Arc::clone(&lines_mtx);

    let (reader_tx, reader_rx) = mpsc::channel::<ReaderThreadMessage>();
    let (term_tx, term_rx) = mpsc::channel::<TerminalThreadMessage>();
    let (input_tx, input_rx) = mpsc::channel::<InputThreadMessage>();

    let term_tx2 = term_tx.clone();
    thread::scope(|scope| {
        scope.spawn(move|| reader_thread_fn(reader_thread_mtx, reader_rx, term_tx, input_reader));
        scope.spawn(move|| term_thread_fn(lines_mtx, reader_tx, term_rx, input_tx));
        scope.spawn(move|| input_thread_fn(term_tx2, input_rx));
    });
}
