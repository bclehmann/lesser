use std::{fs::File, io::{stdout, BufRead, BufReader, Write}, sync::{mpsc, Arc, Mutex}, thread, time::Duration};
use std::io::{Read, Seek};
use std::path::Path;
use std::process::exit;
use crossterm::{
    cursor::{MoveTo, MoveUp}, event::{poll, read, Event, KeyEventKind, KeyModifiers}, execute, queue, style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor}, terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen}
};
use crossterm::terminal::DisableLineWrap;
use clap::Parser;
use notify::{RecursiveMode, Watcher};

#[cfg(unix)]
fn get_tty() -> File {
    File::open("/dev/tty").expect("Could not open /dev/tty")
}

#[cfg(windows)]
fn get_tty() -> File {
    // CON is the equivalent to /dev/tty on Windows
    File::open("CON").expect("Could not open CON")
}

const PAGE_UP_SIZE: usize = 10;

fn print_line(line: &str, highlight: bool) {
    let mut output = stdout();
    if highlight {
        queue!(
                output,
                SetBackgroundColor(Color::Cyan),
                SetForegroundColor(Color::Black),
                Print(line),
                ResetColor
            ).unwrap();
    }else {
        queue!(output, Print(line)).unwrap();
    }
}

fn trim_trailing_newlines(s: &str) -> &str {
    let mut end = s.len();
    for (i, c) in s.char_indices().rev() {
        if c == '\n' || c == '\r' {
            end = i;
        } else {
            break;
        }
    }
    &s[0..end]
}

fn overwrite_last_n_lines(lines: &Vec<String>, pos: Option<usize>, highlight_line_no: Option<usize>) {
    let (cols, rows) = crossterm::terminal::size().expect("Could not get terminal size");
    let mut output = stdout();

    queue!(output, crossterm::terminal::Clear(crossterm::terminal::ClearType::All), MoveTo(0, 0)).unwrap();

    let mut max_displayed_lines = rows;
    let mut start = pos.unwrap_or(
        if lines.len() < rows as usize {
            0
        } else {
            lines.len() - rows as usize + 2
        }
    );

    if lines.len() - start < rows as usize - 1 {
        let old_start = start;
        start = if lines.len() > rows as usize {
            lines.len() - (rows as usize - 1)
        } else {
            0
        };

        let diff = (old_start as isize) - (start as isize);
        max_displayed_lines = rows + diff as u16;
    }

    let mut displayed_lines = 0;
    for i in start..(start + rows as usize - 1) {
        if i >= lines.len() {
            break;
        }
        let mut cur_line = lines[i].as_str();

        while pos.is_none() || displayed_lines < max_displayed_lines as usize - 1 {
            if cur_line.len() > cols as usize {
                print_line(format!("{}\r\n", trim_trailing_newlines(&cur_line[0..cols as usize])).as_str(), highlight_line_no == Some(i));
                cur_line = &cur_line[cols as usize..];
                displayed_lines += 1;
            } else {
                print_line(format!("{}\r\n", trim_trailing_newlines(cur_line)).as_str(), highlight_line_no == Some(i));
                displayed_lines += 1;
                break;
            }
        }
    }

    output.flush().expect("Could not flush output");
}

fn get_matches(lines: &Vec<String>, search: &str, is_regex: bool) -> Vec<usize> {
    let search_as_lower = search.to_lowercase();
    let mut matches = Vec::<usize>::new();

    for (i, line) in lines.iter().enumerate() {
        let as_lower = line.to_lowercase();

        if is_regex {
            if let Ok(re) = regex::Regex::new(search) {
                if re.is_match(line) {
                    matches.push(i);
                }
            }
        } else {
            if as_lower.contains(&search_as_lower) {
                matches.push(i);
            }
        }
    }

    matches
}

fn reader_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, rx: mpsc::Receiver<ReaderThreadMessage>, term_tx: mpsc::Sender<TerminalThreadMessage>, mut input_reader: Box<dyn LineReader>) {
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
            line.pop();
            line.push_str("\r\n");
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

enum ReaderThreadMessage {
    Exit,
}

enum TerminalThreadMessage {
    KeyEvent(crossterm::event::KeyEvent),
    Resize(u16, u16),
    Read
}

enum InputThreadMessage {
    Exit,
}

fn write_status_message(message: &str) {
    let (_, rows) = crossterm::terminal::size().expect("Could not get terminal size");

    execute!(
        stdout(),
        SetBackgroundColor(Color::Grey),
        SetForegroundColor(Color::Black),
        MoveTo(0, rows - 1),
        Clear(ClearType::CurrentLine),
        Print(message),
        ResetColor
    ).unwrap();
}

fn jump_to_match(lines: &Vec<String>, matches: &Vec<usize>, pos: &mut Option<usize>, page_up_size: usize, search: &str, match_regex: bool, match_no: usize) -> Result<(), ()> {
    let mut highlight_line_no = None;

    if match_no < matches.len() {
        *pos = pos_with_in_view(Some(matches[match_no]), page_up_size);
        highlight_line_no = Some(matches[match_no]);
        overwrite_last_n_lines(&lines, *pos, highlight_line_no);

        write_status_message(&format!("Match {}/{} on line {}", match_no + 1, matches.len(), matches[match_no] + 1));
        Ok(())
    } else {
        Err(())
    }
}

// Note, search mode ignores many of the events from term_rx. It has special permission to do so.
fn handle_search_mode(pos: &mut Option<usize>, lines_mtx: &Arc<Mutex<&mut Vec<String>>>, term_rx: &mpsc::Receiver<TerminalThreadMessage>, page_up_size: usize, match_regex: bool) {
    // Is it right to hold the lock for this whole time? Or would the user want to see new results as they come in?
    let lines= lines_mtx.lock().expect("Could not take lock in search event handler");

    let mut highlight_line_no = None;
    let mut search = String::new();

    let prompt = if match_regex { "Regex" } else { "Search" };
    write_status_message(&format!("{}: ", prompt));
    loop {
        match term_rx.recv() {
            Ok(TerminalThreadMessage::KeyEvent(event)) => {
                if event.kind != KeyEventKind::Press {
                    continue;
                }
                match event.code {
                    crossterm::event::KeyCode::Char(c) => {
                        search.push(c);
                    }
                    crossterm::event::KeyCode::Backspace => {
                        if search.len() > 0 {
                            search.pop();
                        } else {
                            overwrite_last_n_lines(&lines, *pos, highlight_line_no);
                            return;
                        }
                    }
                    crossterm::event::KeyCode::Esc => {
                        overwrite_last_n_lines(&lines, *pos, highlight_line_no);
                        return;
                    }
                    crossterm::event::KeyCode::Enter => {
                        break;
                    }
                    _ => {
                    }
                }
            },
            _ => {
                continue;
            }
        }
        let matches = get_matches(&lines, search.trim(), match_regex);
        let _ = jump_to_match(&lines, &matches, pos, page_up_size, search.trim(), match_regex, 0);
        write_status_message(&format!("{}: {}", prompt, search));
    }

    {
        let mut match_no = 0;
        let matches = get_matches(&lines, search.trim(), match_regex);
        let _ = jump_to_match(&lines, &matches, pos, page_up_size, search.trim(), match_regex, match_no);

        loop {
            match term_rx.recv() {
                Ok(TerminalThreadMessage::KeyEvent(event)) => {
                    if event.kind != KeyEventKind::Press {
                        continue;
                    }
                    match event.code {
                        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('q') => {
                            highlight_line_no = None;
                            overwrite_last_n_lines(&lines, *pos, highlight_line_no);
                            break;
                        }
                        crossterm::event::KeyCode::Char('n') | crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Right | crossterm::event::KeyCode::Enter => {
                            match_no = (match_no + 1) % matches.len();
                            let _ = jump_to_match(&lines, &matches, pos, page_up_size, search.trim(), match_regex, match_no);
                        }
                        crossterm::event::KeyCode::Char('p') | crossterm::event::KeyCode::Up |  crossterm::event::KeyCode::Left => {
                            match_no = if match_no > 0 { match_no - 1 } else { matches.len() - 1 };
                            let _ = jump_to_match(&lines, &matches, pos, page_up_size, search.trim(), match_regex, match_no);
                        }
                        _ => {
                        }
                    }
                },
                _ => {
                    continue;
                }
            }
        }
    }
}

fn handle_go_to_line(pos: Option<usize>, n_lines: usize, term_rx: &mpsc::Receiver<TerminalThreadMessage>) -> Option<usize> {
    write_status_message("Go to line: ");
    let mut line_no = String::new();
    loop {
        match term_rx.recv() {
            Ok(TerminalThreadMessage::KeyEvent(event)) => {
                if event.kind != KeyEventKind::Press {
                    continue;
                }
                match event.code {
                    crossterm::event::KeyCode::Char(c) if c.is_numeric() => {
                        line_no.push(c);
                        write_status_message(&format!("Go to line: {}", line_no));
                    }
                    crossterm::event::KeyCode::Char('g') | crossterm::event::KeyCode::Char('G') => {
                        return Some(0);
                    }
                    crossterm::event::KeyCode::Backspace => {
                        if line_no.len() > 0 {
                            line_no.pop();
                        } else {
                            return pos;
                        }
                        write_status_message(&format!("Go to line: {}", line_no));
                    }
                    crossterm::event::KeyCode::Esc => {
                        return pos;
                    }
                    crossterm::event::KeyCode::Enter => {
                        break;
                    }
                    _ => {
                    }
                }
            },
            _ => {
                continue;
            }
        }
    }

    let line_no = line_no.trim();
    if line_no.len() == 0 {
        return pos;
    }

    let line_no = line_no.parse::<usize>().expect("Could not parse line number");
    return if line_no > n_lines {
        None
    } else if line_no == 0 {
        Some(0)
    } else {
        Some(line_no - 1)
    };
}

fn pos_with_in_view(pos: Option<usize>, page_up_size: usize) -> Option<usize> {
    if let Some(n) = pos {
        if n >= page_up_size {
            Some(n - page_up_size)
        } else {
            Some(0)
        }
    } else {
        pos
    }
}

fn get_pos(pos: Option<usize>, n_lines: usize, n_rows: usize, requested_offset: i32) -> Option<usize> {
    if requested_offset == 0 {
        pos
    } else if requested_offset > 0 {
        if let Some(mut n) = pos {
            n += requested_offset as usize;

            if n >= n_lines {
                return None;
            }

            Some(n)
        } else {
            None
        }
    } else {
        if let Some(n) = pos {
            if n < -requested_offset as usize {
                return Some(0);
            }
            Some(n - (-requested_offset as usize))
        } else {
            if n_lines < -requested_offset as usize {
                return Some(0);
            }
            if n_lines < n_rows {
                return Some(n_lines - (-requested_offset as usize));
            }
            return Some(n_lines - n_rows - (-requested_offset as usize))
        }
    }
}

fn page_by(lines: &Vec<String>, pos: &mut Option<usize>, offset: i32) {
    let (_, rows) = crossterm::terminal::size().expect("Could not get terminal size");
    *pos = get_pos(*pos, lines.len(), rows as usize, offset);

    overwrite_last_n_lines(&lines, *pos, None);
}

fn term_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, reader_tx: mpsc::Sender<ReaderThreadMessage>, term_rx: mpsc::Receiver<TerminalThreadMessage>, input_tx: mpsc::Sender<InputThreadMessage>) {
    execute!(stdout(), EnterAlternateScreen).unwrap();
    execute!(stdout(), DisableLineWrap).unwrap();
    let mut pos: Option<usize> = Some(0);

    thread::sleep(Duration::from_millis(100)); // i.e. make sure there's some stuff to read on first draw
    {
        let (_, rows) = crossterm::terminal::size().expect("Could not get terminal size");
        let lines = lines_mtx.lock().expect("Could not take lock in term_thread");
        if lines.len() < rows as usize { // If there aren't many lines we can start in autoscroll
            pos = None;
        }
    }

    enable_raw_mode().expect("Could not enter raw mode");
    
    loop {
        if let Ok(message) = term_rx.recv() {
            match message {
                TerminalThreadMessage::KeyEvent(event) => {
                    if event.kind != KeyEventKind::Press {
                        continue;
                    }

                    match event.code {
                        crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Char('Q') | crossterm::event::KeyCode::Esc => {
                            break;
                        }
                        crossterm::event::KeyCode::Up => {
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in ArrrowUp event handler");
                                page_by(&lines, &mut pos, -1);
                            }
                        }
                        crossterm::event::KeyCode::Char('u') | crossterm::event::KeyCode::Char('U') | crossterm::event::KeyCode::PageUp => {
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in PgUp event handler");
                                page_by(&lines, &mut pos, -(PAGE_UP_SIZE as i32));
                            }
                        }
                        crossterm::event::KeyCode::Down => {
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in ArrowDown event handler");
                                page_by(&lines, &mut pos, 1);
                            }
                        }
                        crossterm::event::KeyCode::Char('d') | crossterm::event::KeyCode::Char('D') | crossterm::event::KeyCode::PageDown | crossterm::event::KeyCode::Char(' ') => {
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in PgDn event handler");
                                page_by(&lines, &mut pos, PAGE_UP_SIZE as i32);
                            }
                        }
                        crossterm::event::KeyCode::Enter => {
                            pos = None;
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in Enter event handler");
                                overwrite_last_n_lines(&lines, pos, None);
                            }
                        }
                        crossterm::event::KeyCode::Char('g') | crossterm::event::KeyCode::Char('G') => {
                            let mut highlight_line_no = None;
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in goto line event handler");
                                if event.modifiers.contains(KeyModifiers::SHIFT) {
                                    pos = None;
                                } else {
                                    let line_no: Option<usize> = handle_go_to_line(pos, lines.len(), &term_rx);
                                    highlight_line_no = line_no;
                                    pos = pos_with_in_view(line_no, PAGE_UP_SIZE);
                                }
                                overwrite_last_n_lines(&lines, pos, highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Char('/') => {
                            handle_search_mode(&mut pos, &lines_mtx, &term_rx, PAGE_UP_SIZE, false);
                        }
                        crossterm::event::KeyCode::Char('r') | crossterm::event::KeyCode::Char('R') => {
                            handle_search_mode(&mut pos, &lines_mtx, &term_rx, PAGE_UP_SIZE, true);
                        }
                        _ => {}
                    }
                },
                TerminalThreadMessage::Resize(_, _) => {
                    let lines = lines_mtx.lock().expect("Could not take lock in resize event handler");
                    overwrite_last_n_lines(&lines, pos, None);
                }
                TerminalThreadMessage::Read => {
                    let lines = lines_mtx.lock().expect("Could not take lock in read event handler");
                    overwrite_last_n_lines(&lines, pos, None);
                }
            }
        }
    }

    execute!(stdout(), LeaveAlternateScreen).unwrap();
    disable_raw_mode().expect("Could not exit raw mode");

    let _ = reader_tx.send(ReaderThreadMessage::Exit); // If it's not received that's ok, that probably means the thread has already exited
    let _ = input_tx.send(InputThreadMessage::Exit); // If it's not received that's ok, that probably means the thread has already exited
    exit(0);
}

fn input_thread_fn(term_tx: mpsc::Sender<TerminalThreadMessage>, input_rx: mpsc::Receiver<InputThreadMessage>) {
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

trait LineReader: Send {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize>;
}

struct StdinReader {
    stdin: std::io::Stdin,
}

impl LineReader for StdinReader {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        self.stdin.read_line(buf)
    }
}

struct FileReader {
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

struct WatchingFileReader {
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
            let stdin = std::io::stdin();
            Box::new(StdinReader { stdin })
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
