use std::fs::File;
use std::io::{BufRead, BufReader, Seek};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use notify::{RecursiveMode, Watcher};

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
    pub fn new(path: &str) -> Self {
        let file = File::open(path).expect("Could not open input file");
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
}

impl WatchingFileReader {
    pub fn new(path: &str) -> Self {
        let file = File::open(path).expect("Could not open input file");
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(tx).expect("Could not create file watcher");

        watcher.watch(Path::new(path), RecursiveMode::NonRecursive).expect("Could not watch file");

        WatchingFileReader {
            file,
            offset: 0,
            rx,
        }
    }
}

impl LineReader for WatchingFileReader {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        while self.file.metadata()?.len() <= self.offset as u64 {
            for res in &self.rx { // A normal recv immediately returns an error. Perhaps a sender and receiver on the same thread is not a good idea?
                match res {
                    Ok(notify::Event { .. }) => {
                        break;
                    }
                    Err(_) => {
                        return Err(std::io::Error::new(std::io::ErrorKind::Other, "File watch channel disconnected"));
                    }
                }
            }

            thread::sleep(Duration::from_millis(250));
        }

        let mut reader = BufReader::new(&self.file);
        reader.seek(std::io::SeekFrom::Start(self.offset as u64))?;
        let n = reader.read_line(buf)?;
        self.offset += n;
        Ok(n)
    }
}
