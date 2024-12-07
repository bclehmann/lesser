use std::{fs::File, io::{stdout, BufRead, BufReader, Read, Write}, time::Duration};
use crossterm::{
    event::{self, poll, read, Event, KeyEventKind}, execute, queue, style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor}, terminal::{ScrollDown, ScrollUp}, ExecutableCommand
};

#[cfg(unix)]
fn get_tty() {
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

fn main() -> std::io::Result<()> {
    let mut from_end: usize = 0;
    let mut highlight_line_no: Option<usize> = None;

    let mut reader = BufReader::new(std::io::stdin());
    let mut lines = Vec::<String>::new();
    let mut line = String::new();
    while let Ok(n) = reader.read_line(&mut line) {
        if n == 0 {
            break;
        }

        lines.push(line.clone());
        line.clear();
    }

    overwrite_last_n_lines(&lines, from_end, highlight_line_no);

    let tty = get_tty();
    let mut tty_reader = BufReader::new(tty);

    let (_, mut rows) = crossterm::terminal::size().expect("Could not get terminal size");

    loop {
        // read is guaranteed not to block when poll returns Ok(true)
        if poll(Duration::MAX)? {
            match read()? {
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

                            if from_end + rows as usize > lines.len() {
                                from_end = lines.len() - rows as usize;
                            }

                            highlight_line_no = None;
                            overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                        }
                        crossterm::event::KeyCode::Char('d') | crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char(' ') |  crossterm::event::KeyCode::Enter => {
                            highlight_line_no = None;
                            if from_end == 0 {
                                continue;
                            }
                            from_end -= 1;

                            if from_end + rows as usize > lines.len() {
                                from_end = lines.len() - rows as usize;
                            }

                            overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                        }
                        crossterm::event::KeyCode::Char('/') => {
                            highlight_line_no = None;
                            let mut search = String::new();
                            // It's much easier to do this than to do the same thing through crossterm events
                            tty_reader.read_line(&mut search).expect("Could not read search string");

                            let mut search_result = first_instance_of_term_past(&lines, search.trim(), 0);
                            loop {
                                match search_result {
                                    Some(i) => {
                                        highlight_line_no = Some(i);
                                        from_end = lines.len() - i - 1;
                                        overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                                    }
                                    None => {
                                        highlight_line_no = None;
                                        overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                                        break;
                                    }
                                }
                                match(read()?) {
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
                        _ => {}
                    }
                },
                Event::Resize(_, n) => {
                    rows = n;
                    overwrite_last_n_lines(&lines, from_end, highlight_line_no);
                }
                _ => {}
            }
        } else {
            // Timeout expired and no `Event` is available
        }
    }

    Ok(())
}
