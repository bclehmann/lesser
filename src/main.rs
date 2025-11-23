mod terminal;
mod messaging;
mod input;
mod reader;

use std::{fs::File, io::{BufRead, Write}, sync::{mpsc, Arc, Mutex}, thread};
use std::cell::Cell;
use std::io::{Read, Seek};
use clap::Parser;
use notify::{Watcher};
use crate::input::input_thread_fn;
use crate::messaging::{TerminalThreadMessage};
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
    filenames: Option<Vec<String>>,

    #[arg(long)]
    watch: bool,
}

pub struct Source {
    name: String,
    reader: Mutex<Box<dyn LineReader>>,
    lines: Mutex<Vec<String>>,
}

fn main() {
    let args = Args::parse();

    let mut sources: Vec<Arc<Source>> = match &args.filenames {
        Some(filenames) => {
            filenames.iter().map(|fname| {
                if !args.watch {
                    Arc::new(
                        Source {
                            name: fname.clone(),
                            reader: Mutex::new(Box::new(FileReader::new(fname)) as Box<dyn LineReader>),
                            lines: Mutex::new(Vec::<String>::new()),
                        }
                    )
                } else {
                    Arc::new(
                        Source {
                            name: fname.clone(),
                            reader: Mutex::new(Box::new(WatchingFileReader::new(fname)) as Box<dyn LineReader>),
                            lines: Mutex::new(Vec::<String>::new()),
                        }
                    )
                }
            }).collect()
        }
        None => {
            vec!(
                Arc::new(
                    Source {
                        name: "stdin".to_string(),
                        reader: Mutex::new(Box::new(StdinReader::new())),
                        lines: Mutex::new(Vec::<String>::new()),
                    }
                )
            )
        }
    };

    let (term_tx, term_rx) = mpsc::channel::<TerminalThreadMessage>();

    let term_tx2 = term_tx.clone();
    thread::scope(|scope| {
        for source in sources.iter() {
            let source = source.clone();
            let term_tx = term_tx.clone();
            scope.spawn(move|| reader_thread_fn(source, term_tx));
        }

        scope.spawn(move|| term_thread_fn(&sources, term_rx));
        scope.spawn(move|| input_thread_fn(term_tx2));
    });
}
