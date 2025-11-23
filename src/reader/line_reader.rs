use std::fs::File;
use std::io::{BufRead, BufReader, Seek};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use notify::{Config, RecursiveMode, Watcher};

pub trait LineReader: Send {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize>;
}

pub struct StdinReader {
    stdin: std::io::Stdin,
}

impl StdinReader {
    pub fn new() -> Self {
        StdinReader {
            stdin: std::io::stdin(),
        }
    }
}

impl LineReader for StdinReader {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        self.stdin.read_line(buf)
    }
}

pub struct FileReader {
    reader: BufReader<File>,
}

impl FileReader {
    pub fn new(file: File) -> Self {
        let reader = BufReader::new(file);

        FileReader {
            reader,
        }
    }
}

impl LineReader for FileReader {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        self.reader.read_line(buf)
    }
}

pub struct WatchingFileReader {
    file: File,
    offset: usize,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    watcher: notify::PollWatcher,
}

impl WatchingFileReader {
    pub fn new(file: File, path: &str) -> Self {
        let (watcher_tx, watcher_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();

        // I've been having difficulties with recommended_watcher so polling it is :/
        let mut watcher = notify::PollWatcher::new(watcher_tx,Config::default().with_poll_interval(Duration::from_millis(500))).expect("Could not create file watcher");
        watcher.watch(Path::new(path), RecursiveMode::NonRecursive).expect("Could not watch file");

        thread::spawn(move || {
            let msg = watcher_rx.recv();
            if let Ok(v) = msg {
                tx.send(v).expect("Could not send file change event");
            } else {
                eprintln!("File watcher error: {:?}", msg);
            }
        });


        WatchingFileReader {
            file,
            offset: 0,
            rx,
            watcher,
        }
    }
}

impl LineReader for WatchingFileReader {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        while self.file.metadata()?.len() <= self.offset as u64 {
            match &self.rx.recv() {
                Ok(_) => {
                    eprintln!("File changed!");
                    continue;
                }
                Err(e) => {
                    eprintln!("Error: {:?}", e);
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                }
            }
        }

        let mut reader = BufReader::new(&self.file);
        reader.seek(std::io::SeekFrom::Start(self.offset as u64))?;
        let n = reader.read_line(buf)?;
        self.offset += n;
        Ok(n)
    }
}
