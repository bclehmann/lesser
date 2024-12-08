use std::{fs::File, io::{stdout, BufRead, BufReader, Read, Write}, sync::{mpsc, Arc, Mutex}, thread, time::Duration};
use crossterm::{
    event::{self, poll, read, Event, KeyEventKind}, execute, queue, style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor}, terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, ScrollDown, ScrollUp}, ExecutableCommand
};
use libc::{getchar, EOF};

#[cfg(unix)]
fn get_tty() -> File {
    File::open("/dev/tty").expect("Could not open /dev/tty")
}

#[cfg(windows)]
fn get_tty() -> File {
    // CON is the equivalent to /dev/tty on Windows
    File::open("CON").expect("Could not open CON")
}

fn overwrite_last_n_lines(lines: &Vec<String>, from_end: usize, highlight_line_no: Option<usize>) {
    let (_, rows) = crossterm::terminal::size().expect("Could not get terminal size");
    let mut output = stdout();

    queue!(output, crossterm::terminal::Clear(crossterm::terminal::ClearType::All)).unwrap();

    let offset = from_end + rows as usize;
    let start = if offset > lines.len() {
        0
    } else {
        lines.len() - offset
    };

    for i in start..(start + rows as usize) {
        if i >= lines.len() {
            break;
        }

        match highlight_line_no {
            Some(j) if i == j => {
                queue!(
                    output,
                    SetBackgroundColor(Color::White),
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

    output.flush();
}

fn first_instance_of_term_past(lines: &Vec<String>, search: &str, start: usize) -> Option<usize> {
    for (i, line) in lines.iter().skip(start).enumerate() {
        if line.contains(search) {
            return Some(i + start);
        }
    }

    None
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

fn term_thread_fn(lines_mtx: Arc<Mutex<&mut Vec<String>>>, tx: mpsc::Sender<ThreadMessage>) {
    execute!(stdout(), EnterAlternateScreen).unwrap();
    let mut from_end: usize = 0;
    let mut highlight_line_no: Option<usize> = None;
    let mut last_line_length: i32= -1;

    {
        let lines = lines_mtx.lock().expect("Could not take lock in term_thread");
        if lines.len() != last_line_length as usize {
            last_line_length = lines.len() as i32;
            overwrite_last_n_lines(&lines, from_end, highlight_line_no);
        }
    }

    let tty = get_tty();
    let mut tty_reader = BufReader::new(tty);
    let (_, mut rows) = crossterm::terminal::size().expect("Could not get terminal size");

    #[cfg(unix)]
    {
        enable_raw_mode().expect("Could not enter raw mode");
    }
    
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
                            from_end += 1;

                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in KeyUp event handler");

                                if from_end + rows as usize > lines.len() {
                                    from_end = lines.len() - rows as usize;
                                }

                                highlight_line_no = None;
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Char('d') | crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char(' ') |  crossterm::event::KeyCode::Enter => {
                            highlight_line_no = None;
                            if from_end == 0 {
                                continue;
                            }
                            from_end -= 1;

                            {
                                let lines = lines_mtx.lock().expect("Could not take lock in KeyDown event handler");

                                if from_end + rows as usize > lines.len() {
                                    from_end = lines.len() - rows as usize;
                                }
    
                                last_line_length = lines.len() as i32;
                                overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Char('/') => {
                            highlight_line_no = None;
                            let mut search = String::new();
                            // It's much easier to do this than to do the same thing through crossterm events
                            tty_reader.read_line(&mut search).expect("Could not read search string");

                            {
                                // Is it right to hold the lock for this whole time? Or would the user want to see new results as they come in?
                                let lines= lines_mtx.lock().expect("Could not take lock in search event handler");

                                let mut search_result = first_instance_of_term_past(&lines, search.trim(), 0);
                                loop {
                                    match search_result {
                                        Some(i) => {
                                            highlight_line_no = Some(i);
                                            from_end = lines.len() - i - 1;
                                            last_line_length = lines.len() as i32;
                                            overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                                        }
                                        None => {
                                            highlight_line_no = None;
                                            last_line_length = lines.len() as i32;
                                            overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                                            break;
                                        }
                                    }
                                    match read().unwrap() {
                                        Event::Key(event) => {
                                            if event.kind != KeyEventKind::Press {
                                                continue;
                                            }
                                            match event.code {
                                                crossterm::event::KeyCode::Enter => {
                                                    search_result = first_instance_of_term_past(&lines, search.trim(), highlight_line_no.unwrap() + 1);
                                                }
                                                _ => {
                                                    highlight_line_no = None;
                                                    last_line_length = lines.len() as i32;
                                                    overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                                                    break;
                                                },
                                            }
                                        },
                                        _ => {
                                            continue;
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
                        overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                    }
                }
                _ => {}
            }
        } else {
            {
                let lines = lines_mtx.lock().expect("Could not take lock in resize event handler");
                if lines.len() != last_line_length as usize {
                    last_line_length = lines.len() as i32;
                    overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                }
            }
        }
    }

    #[cfg(unix)]
    {
        disable_raw_mode().expect("Could not exit raw mode");
    }

    execute!(stdout(), LeaveAlternateScreen).unwrap();

    let _ = tx.send(ThreadMessage::Exit); // If it's not received that's ok, that probably means the reader thread has already exited
}

fn main() {
    let mut lines_raw = Vec::<String>::new();
    let lines_mtx = Arc::new(Mutex::new(&mut lines_raw));
    let reader_thread_mtx = Arc::clone(&lines_mtx);

    let (tx, rx) = mpsc::channel::<ThreadMessage>();

    thread::scope(|scope| {
        let render_thread = scope.spawn(move|| reader_thread_fn(reader_thread_mtx, rx));
        let term_thread = scope.spawn(move|| term_thread_fn(lines_mtx, tx));

        term_thread.join().expect("Could not join term_thread");
    });
}
