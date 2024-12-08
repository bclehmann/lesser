use std::{fs::File, io::{stdout, BufRead, BufReader, Write}, sync::{mpsc, Arc, Mutex}, thread, time::Duration};
use crossterm::{
    cursor::{MoveTo, MoveUp}, event::{poll, read, Event, KeyEventKind}, execute, queue, style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor}, terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen}
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

fn overwrite_last_n_lines(lines: &Vec<String>, pos: Option<usize>, highlight_line_no: Option<usize>) {
    let (_, rows) = crossterm::terminal::size().expect("Could not get terminal size");
    let mut output = stdout();

    queue!(output, crossterm::terminal::Clear(crossterm::terminal::ClearType::All), MoveTo(0, 0)).unwrap();

    let start = if let Some(n) = pos {
        if n + rows as usize > lines.len() {
            lines.len() - rows as usize
        } else {
            n
        }
    } else { 
        if lines.len() < rows as usize {
            0
        } else {
            lines.len() - rows as usize
        }
    };

    for i in start..(start + rows as usize - 1) {
        if i >= lines.len() {
            break;
        }

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

fn get_matches(lines: &Vec<String>, search: &str) -> Vec<usize> {
    let mut matches = Vec::<usize>::new();
    for (i, line) in lines.iter().enumerate() {
        if line.contains(search) {
            matches.push(i);
        }
    }

    matches
}

fn reader_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, rx: mpsc::Receiver<ThreadMessage>) {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    while let Ok(n) = reader.read_line(&mut line) {
        if let Ok(message) = rx.try_recv() {
            match message {
                ThreadMessage::Exit => {
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
    }
}

enum ThreadMessage {
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

fn term_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, tx: mpsc::Sender<ThreadMessage>) {
    execute!(stdout(), EnterAlternateScreen).unwrap();
    let mut pos: Option<usize> = None;
    let mut highlight_line_no: Option<usize> = None;
    let mut last_line_length: i32= -1;

    {
        let lines = lines_mtx.lock().expect("Could not take lock in term_thread");
        if lines.len() != last_line_length as usize {
            last_line_length = lines.len() as i32;
            overwrite_last_n_lines(&lines, pos, highlight_line_no);
        }
    }
    let (_, mut rows) = crossterm::terminal::size().expect("Could not get terminal size");

    enable_raw_mode().expect("Could not enter raw mode");
    
    loop {
        // read is guaranteed not to block when poll returns Ok(true)
        if poll(Duration::from_millis(100)).unwrap() {
            match read().unwrap() {
                Event::Key(event) => {
                    if event.kind != KeyEventKind::Press {
                        continue;
                    }

                    match event.code {
                        crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Esc => {
                            break;
                        }
                        crossterm::event::KeyCode::Char('u') | crossterm::event::KeyCode::Up => {
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in KeyUp event handler");

                                if let Some(n) = pos {
                                    if n > 0 {
                                        pos = Some(n - 1);
                                    }
                                } else {
                                    pos = Some(lines.len() - rows as usize - 1);
                                }

                                highlight_line_no = None;
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, pos, highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Char('d') | crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char(' ') => {
                            highlight_line_no = None;

                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in KeyDown event handler");
                                if let Some(n) = pos {
                                    if n < lines.len() - rows as usize {
                                        pos = Some(n + 1);
                                    } else {
                                        pos = None;
                                    }
                                }
    
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, pos, highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Enter => {
                            pos = None;
                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in Enter event handler");
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, pos, highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Char('/') => {
                            highlight_line_no = None;
                            let mut search = String::new();
                            write_status_message("Query: ");
                            loop {
                                match read().unwrap() {
                                    Event::Key(event) => {
                                        if event.kind != KeyEventKind::Press {
                                            continue;
                                        }
                                        match event.code {
                                            crossterm::event::KeyCode::Char(c) => {
                                                search.push(c);
                                                write_status_message(&format!("Query: {}", search));
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

                                let matches = get_matches(&lines, search.trim());
                                if matches.len() == 0 {
                                    write_status_message("No matches found");
                                } else {
                                    pos = Some(matches[0]);
                                    highlight_line_no = Some(matches[0]);
                                    last_line_length = lines.len() as i32;
                                    overwrite_last_n_lines(&lines, pos, highlight_line_no);

                                    let mut match_no = 0;
                                    write_status_message(&format!("Match {}/{} on line {}", match_no + 1, matches.len(), matches[match_no] + 1));

                                    loop {
                                        match read().unwrap() {
                                            Event::Key(event) => {
                                                if event.kind != KeyEventKind::Press {
                                                    continue;
                                                }
                                                match event.code {
                                                    crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('q') => {
                                                        highlight_line_no = None;
                                                        overwrite_last_n_lines(&lines, pos, highlight_line_no);
                                                        break;
                                                    }
                                                    crossterm::event::KeyCode::Char('n') | crossterm::event::KeyCode::Down => {
                                                        match_no = (match_no + 1) % matches.len();

                                                        pos = Some(matches[match_no]);
                                                        highlight_line_no = Some(matches[match_no]);
                                                        last_line_length = lines.len() as i32;
                                                        overwrite_last_n_lines(&lines, pos, highlight_line_no);

                                                        write_status_message(&format!("Match {}/{} on line {}", match_no + 1, matches.len(), matches[match_no] + 1));
                                                    }
                                                    crossterm::event::KeyCode::Char('p') | crossterm::event::KeyCode::Up => {
                                                        match_no = if match_no > 0 { match_no - 1 } else { matches.len() - 1 };

                                                        pos = Some(matches[match_no]);
                                                        highlight_line_no = Some(matches[match_no]);
                                                        last_line_length = lines.len() as i32;
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
                        _ => {}
                    }
                },
                Event::Resize(_, n) => {
                    rows = n;
                    {
                        let lines = lines_mtx.lock().expect("Could not take lock in resize event handler");
                        last_line_length = lines.len() as i32;
                        overwrite_last_n_lines(&lines, pos, highlight_line_no);
                    }
                }
                _ => {}
            }
        } else {
            {
                let lines = lines_mtx.lock().expect("Could not take lock in resize event handler");
                if lines.len() != last_line_length as usize {
                    last_line_length = lines.len() as i32;
                    overwrite_last_n_lines(&lines, pos, highlight_line_no);
                }
            }
        }
    }

    execute!(stdout(), LeaveAlternateScreen).unwrap();
    disable_raw_mode().expect("Could not exit raw mode");

    let _ = tx.send(ThreadMessage::Exit); // If it's not received that's ok, that probably means the reader thread has already exited
}

fn main() {
    let mut lines_raw = Vec::<String>::new();
    let lines_mtx = Arc::new(Mutex::new(&mut lines_raw));
    let reader_thread_mtx = Arc::clone(&lines_mtx);

    let (tx, rx) = mpsc::channel::<ThreadMessage>();

    thread::scope(|scope| {
        scope.spawn(move|| reader_thread_fn(reader_thread_mtx, rx));
        scope.spawn(move|| term_thread_fn(lines_mtx, tx));
    });
}
