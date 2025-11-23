mod terminal;
mod messaging;
mod input;
mod reader;

use std::{fs::File, io::{BufRead, Write}, sync::{mpsc, Arc, Mutex}, thread};
use std::fs::metadata;
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

pub trait Source: Send + Sync {
    fn get_name(&self) -> &str;
    fn get_reader(&self) -> Option<&Mutex<Box<dyn LineReader>>>;
    fn get_lines(&self) -> &Mutex<Vec<String>>;
    fn add_line(&self, line: String) -> Result<(), String>;
    fn add_listener(&self, listener: Arc<dyn Source>) -> Result<(), String> {
        Err("This source does not support listeners".to_string())
    }
}

pub struct AggregateSource {
    name: String,
    lines: Mutex<Vec<String>>,
}

impl Source for AggregateSource {
    fn get_name(&self) -> &str {
        self.name.as_str()
    }

    fn get_reader(&self) -> Option<&Mutex<Box<dyn LineReader>>> {
        None
    }

    fn get_lines(&self) -> &Mutex<Vec<String>> {
        &self.lines
    }

    fn add_line(&self, line: String) -> Result<(), String> {
        let mut lines = self.lines.lock().map_err(|e| format!("Could not lock lines mutex: {}", e))?;
        lines.push(line);
        Ok(())
    }
}

pub struct ReadableSource {
    name: String,
    reader: Mutex<Box<dyn LineReader>>,
    lines: Mutex<Vec<String>>,

    listeners: Mutex<Vec<Arc<dyn Source>>>,
}

impl ReadableSource {
    pub fn new(name: String, reader: Box<dyn LineReader>) -> Self {
        ReadableSource {
            name,
            reader: Mutex::new(reader),
            lines: Mutex::new(Vec::<String>::new()),
            listeners: Mutex::new(Vec::<Arc<dyn Source>>::new()),
        }
    }
}

impl Source for ReadableSource {
    fn get_name(&self) -> &str {
        self.name.as_str()
    }

    fn get_reader(&self) -> Option<&Mutex<Box<dyn LineReader>>> {
        Some(&self.reader)
    }

    fn get_lines(&self) -> &Mutex<Vec<String>> {
        &self.lines
    }

    fn add_line(&self, line: String) -> Result<(), String> {
        {
            let mut lines = self.lines.lock().map_err(|e| format!("Could not lock lines mutex: {}", e))?;
            lines.push(line.clone());
        }

        let listeners = self.listeners.lock().map_err(|e| format!("Could not lock listeners mutex: {}", e))?;
        for listener in listeners.iter() {
            listener.add_line(line.clone())?;
        }

        Ok(())
    }

    fn add_listener(&self, listener: Arc<dyn Source>) -> Result<(), String> {
        let mut listeners = self.listeners.lock().map_err(|e| format!("Could not lock listeners mutex: {}", e))?;
        listeners.push(listener);
        Ok(())
    }
}

fn main() {
    let args = Args::parse();

    let mut sources: Vec<Arc<dyn Source>> = match &args.filenames {
        Some(filenames) => {
            filenames.iter().flat_map(|pattern| glob::glob(pattern).expect("Could not create glob").into_iter()).map(|path| {
                let fname = path.expect("Could not read globbed path").to_string_lossy().to_string();
                let file = File::open(fname.as_str()).expect("Could not open input file");
                if file.metadata().expect("Could not read metadata").is_dir() {
                    return None;
                }

                if !args.watch {
                    Some(
                            Arc::new(
                                ReadableSource::new(
                                    fname,
                                    Box::new(FileReader::new(file)) as Box<dyn LineReader>
                                )
                        ) as Arc<dyn Source>
                    )
                } else {
                    Some(
                            Arc::new(
                                ReadableSource::new(
                                    fname.clone(),
                                    Box::new(WatchingFileReader::new(file, fname.as_str())) as Box<dyn LineReader>
                                )
                        ) as Arc<dyn Source>
                    )
                }
            }).filter(|s| s.is_some()).map(|s| s.unwrap()).collect()
        }
        None => {
            vec!(
                Arc::new(
                    ReadableSource::new(
                        "stdin".to_string(),
                        Box::new(StdinReader::new())
                    )
                ) as Arc<dyn Source>
            )
        }
    };

    let mut terminal_sources = sources.iter().map(|s| s.clone()).collect::<Vec<Arc<dyn Source>>>();

    if sources.len() == 0 {
        eprintln!("No valid input sources");
        std::process::exit(1);
    } else if sources.len() > 1 {
        let aggregate = Arc::new(
            AggregateSource {
                name: "aggregate".to_string(),
                lines: Mutex::new(Vec::<String>::new()),
            }
        ) as Arc<dyn Source>;

        for source in sources.iter() {
            source.add_listener(aggregate.clone()).expect("Could not add aggregate as listener");
        }

        terminal_sources.insert(0, aggregate);
    }

    let (term_tx, term_rx) = mpsc::channel::<TerminalThreadMessage>();

    let term_tx2 = term_tx.clone();
    thread::scope(|scope| {
        for source in sources.iter() {
            let source = source.clone();
            let term_tx = term_tx.clone();
            scope.spawn(move|| reader_thread_fn(source, term_tx));
        }

        scope.spawn(move|| term_thread_fn(&terminal_sources, term_rx));
        scope.spawn(move|| input_thread_fn(term_tx2));
    });
}
