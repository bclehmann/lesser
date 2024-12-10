use std::{fs::File, io::{stdout, BufRead, BufReader, Write}, sync::{mpsc, Arc, Mutex}, thread, time::Duration};
use crossterm::{
    cursor::{MoveTo, MoveUp}, event::{poll, read, Event, KeyEventKind, KeyModifiers}, execute, queue, style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor}, terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen}
};

#[cfg(unix)]
fn get_tty() -> File {
    File::open("/dev/tty").expect("Could not open /dev/tty")
}

#[cfg(windows)]
fn get_tty() -> File {
    // CON is the equivalent to /dev/tty on Windows
    File::open("CON").expect("Could not open CON")
}

fn get_row_count(lines: &[String], cols: usize) -> usize {
    let mut count = 0;
    for line in lines {
        count += get_line_row_count(line, cols);
    }
    count
}

fn get_line_row_count(line: &String, cols: usize) -> usize {
    (line.len() - 2) / cols + 1
}

fn overwrite_last_n_lines(lines: &Vec<String>, pos: &mut Option<usize>, highlight_line_no: Option<usize>) {
    let (cols, rows) = crossterm::terminal::size().expect("Could not get terminal size");
    let mut output = stdout();

    queue!(output, crossterm::terminal::Clear(crossterm::terminal::ClearType::All), MoveTo(0, 0)).unwrap();

    let mut start: usize = if let Some(n) = *pos {
        if n + rows as usize > lines.len() {
            if lines.len() < rows as usize {
                0
            } else {
                lines.len() - rows as usize + 2
            }
        } else {
            n
        }
    } else { 
        if lines.len() < rows as usize {
            0
        } else {
            let mut n = lines.len() - rows as usize - 2;
            while get_row_count(&lines[n..], cols as usize) > rows as usize - 2
            {
                n += 1;
            }
            n
        }
    };

    if let Some(h) = highlight_line_no {
        if h <= start {
            start = h;
            let mut prelude_rows = 0;
            while prelude_rows < 5 {
                if start == 0 {
                    break;
                }
                start -= 1;
                prelude_rows += get_line_row_count(&lines[start], cols as usize);
            }

            if prelude_rows > 10 {
                start += 1;
            }

            *pos = Some(start);
        }
    }

    let mut cumulative_rows = 0;

    for i in start..(start + rows as usize - 1) {
        if i >= lines.len() || cumulative_rows >= rows as usize {
            break;
        }

        cumulative_rows += get_line_row_count(&lines[i], cols as usize);

        match highlight_line_no {
            Some(j) if i == j => {
                queue!(
                    output,
                    SetBackgroundColor(Color::Cyan),
                    SetForegroundColor(Color::Black),
                    Print(lines[i].clone()),
                    ResetColor
                ).unwrap();
            }
            _ => {
                queue!(output, Print(lines[i].clone())).unwrap();
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

fn reader_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, rx: mpsc::Receiver<ReaderThreadMessage>, term_tx: mpsc::Sender<TerminalThreadMessage>) {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    while let Ok(n) = reader.read_line(&mut line) {
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

// Note, search mode ignores many of the events from term_rx. It has special permission to do so.
fn handle_search_mode(pos: &mut Option<usize>, lines_mtx: &Arc<Mutex<&mut Vec<String>>>, term_rx: &mpsc::Receiver<TerminalThreadMessage>, page_up_size: usize, match_regex: bool) {
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
                        write_status_message(&format!("{}: {}", prompt, search));
                    }
                    crossterm::event::KeyCode::Backspace => {
                        if search.len() > 0 {
                            search.pop();
                        } else {
                            return;
                        }
                        write_status_message(&format!("{}: {}", prompt, search));
                    }
                    crossterm::event::KeyCode::Esc => {
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
    }

    {
        // Is it right to hold the lock for this whole time? Or would the user want to see new results as they come in?
        let lines= lines_mtx.lock().expect("Could not take lock in search event handler");

        let matches = get_matches(&lines, search.trim(), match_regex);
        if matches.len() == 0 {
            write_status_message("No matches found");
        } else {
            *pos = Some(matches[0]);
            highlight_line_no = Some(matches[0]);
            overwrite_last_n_lines(&lines, pos, highlight_line_no);

            let mut match_no = 0;
            write_status_message(&format!("Match {}/{} on line {}", match_no + 1, matches.len(), matches[match_no] + 1));

            loop {
                match term_rx.recv() {
                    Ok(TerminalThreadMessage::KeyEvent(event)) => {
                        if event.kind != KeyEventKind::Press {
                            continue;
                        }
                        match event.code {
                            crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('q') => {
                                highlight_line_no = None;
                                overwrite_last_n_lines(&lines, pos, highlight_line_no);
                                break;
                            }
                            crossterm::event::KeyCode::Char('n') | crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Right | crossterm::event::KeyCode::Enter => {
                                match_no = (match_no + 1) % matches.len();

                                *pos = Some(matches[match_no]);
                                highlight_line_no = Some(matches[match_no]);
                                overwrite_last_n_lines(&lines, pos, highlight_line_no);

                                write_status_message(&format!("Match {}/{} on line {}", match_no + 1, matches.len(), matches[match_no] + 1));
                            }
                            crossterm::event::KeyCode::Char('p') | crossterm::event::KeyCode::Up |  crossterm::event::KeyCode::Left => {
                                match_no = if match_no > 0 { match_no - 1 } else { matches.len() - 1 };

                                *pos = Some(matches[match_no]);
                                highlight_line_no = Some(matches[match_no]);
                                overwrite_last_n_lines(&lines, pos, highlight_line_no);

                                write_status_message(&format!("Match {}/{} on line {}", match_no + 1, matches.len(), matches[match_no] + 1));
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

fn get_pos(pos: Option<usize>, n_lines: usize, n_rows: usize, requested_offset: i32) -> Option<usize> {
    if n_lines < n_rows {
        return None;
    }

    if requested_offset == 0 {
        return pos;
    } else if requested_offset > 0 {
        if let Some(mut n) = pos {
            n += requested_offset as usize;

            if n > n_lines - n_rows {
                return None;
            }

            return Some(n);
        } else {
            return None;
        }
    } else {
        if let Some(n) = pos {
            if n < -requested_offset as usize {
                return Some(0);
            }
            return Some(n - (-requested_offset as usize));
        } else {
            if n_lines < n_rows {
                return None;
            } else if n_lines - n_rows < -requested_offset as usize {
                return Some(0);
            } else {
                return Some(n_lines - n_rows - (-requested_offset as usize));
            }
        }
    }
}

fn page_by(lines: &Vec<String>, pos: &mut Option<usize>, offset: i32) {
    let (cols, rows) = crossterm::terminal::size().expect("Could not get terminal size");
    *pos = get_pos(*pos, get_row_count(lines, cols as usize), rows as usize, offset);

    overwrite_last_n_lines(&lines, pos, None);
}

fn term_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, reader_tx: mpsc::Sender<ReaderThreadMessage>, term_rx: mpsc::Receiver<TerminalThreadMessage>, input_tx: mpsc::Sender<InputThreadMessage>) {
    execute!(stdout(), EnterAlternateScreen).unwrap();
    let mut pos: Option<usize> = Some(0);
    let mut last_line_length: i32= -1;

    let page_up_size: usize = 10;
    
    thread::sleep(Duration::from_millis(100)); // i.e. make sure there's some stuff to read on first draw
    {
        let (_, rows) = crossterm::terminal::size().expect("Could not get terminal size");
        let lines = lines_mtx.lock().expect("Could not take lock in term_thread");
        if lines.len() < rows as usize { // If there aren't many lines we can start in autoscroll
            pos = None;
        }

        if lines.len() != last_line_length as usize {
            last_line_length = lines.len() as i32;
            overwrite_last_n_lines(&lines, &mut pos, None);
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
                                page_by(&lines, &mut pos, -(page_up_size as i32));
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
                                page_by(&lines, &mut pos, page_up_size as i32);
                            }
                        }
                        crossterm::event::KeyCode::Enter => {
                            pos = None;
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in Enter event handler");
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, &mut pos, None);
                            }
                        }
                        crossterm::event::KeyCode::Char('g') | crossterm::event::KeyCode::Char('G') => {
                            let mut highlight_line_no = None;
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in goto line event handler");
                                if event.modifiers.contains(KeyModifiers::SHIFT) {
                                    pos = None;
                                } else {
                                    let (cols, _) = crossterm::terminal::size().expect("Could not get terminal size");
                                    pos = handle_go_to_line(pos, get_row_count(&lines, cols as usize), &term_rx);
                                    highlight_line_no = pos;
                                }
                                overwrite_last_n_lines(&lines, &mut pos, highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Char('/') => {
                            handle_search_mode(&mut pos, &lines_mtx, &term_rx, page_up_size, false);
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in search event handler");
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, &mut pos, None);
                            }
                        }
                        crossterm::event::KeyCode::Char('r') | crossterm::event::KeyCode::Char('R') => {
                            handle_search_mode(&mut pos, &lines_mtx, &term_rx, page_up_size, true);
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in search event handler");
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, &mut pos, None);
                            }
                        }
                        _ => {}
                    }
                },
                TerminalThreadMessage::Resize(_, _) => {

                }
                TerminalThreadMessage::Read => {
                    {
                        let lines = lines_mtx.lock().expect("Could not take lock in read event handler");

                        if lines.len() != last_line_length as usize {
                            last_line_length = lines.len() as i32;
                            overwrite_last_n_lines(&lines, &mut pos, None);
                        }
                    }
                }
            }
        }
    }

    execute!(stdout(), LeaveAlternateScreen).unwrap();
    disable_raw_mode().expect("Could not exit raw mode");

    let _ = reader_tx.send(ReaderThreadMessage::Exit); // If it's not received that's ok, that probably means the thread has already exited
    let _ = input_tx.send(InputThreadMessage::Exit); // If it's not received that's ok, that probably means the thread has already exited
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

fn main() {
    let mut lines_raw = Vec::<String>::new();
    let lines_mtx = Arc::new(Mutex::new(&mut lines_raw));
    let reader_thread_mtx = Arc::clone(&lines_mtx);

    let (reader_tx, reader_rx) = mpsc::channel::<ReaderThreadMessage>();
    let (term_tx, term_rx) = mpsc::channel::<TerminalThreadMessage>();
    let (input_tx, input_rx) = mpsc::channel::<InputThreadMessage>();

    let term_tx2 = term_tx.clone();
    thread::scope(|scope| {
        scope.spawn(move|| reader_thread_fn(reader_thread_mtx, reader_rx, term_tx));
        scope.spawn(move|| term_thread_fn(lines_mtx, reader_tx, term_rx, input_tx));
        scope.spawn(move|| input_thread_fn(term_tx2, input_rx));
    });
}
