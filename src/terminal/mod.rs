use std::io::{stdout, Write};
use std::process::exit;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use crossterm::event::{KeyEventKind, KeyModifiers};
use crossterm::{execute, queue};
use crossterm::cursor::MoveTo;
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen};
use crate::{Source, TerminalThreadMessage};

const PAGE_UP_SIZE: usize = 10;

pub fn term_thread_fn(sources: &[Arc<Source>], term_rx: mpsc::Receiver<TerminalThreadMessage>) {
    execute!(stdout(), EnterAlternateScreen).unwrap();
    execute!(stdout(), DisableLineWrap).unwrap();

    let mut pos_by_source = sources.iter().map(|_| Some(0)).collect::<Vec<Option<usize>>>();
    let mut source_index = 0;

    thread::sleep(Duration::from_millis(100)); // i.e. make sure there's some stuff to read on first draw
    {
        let (_, rows) = crossterm::terminal::size().expect("Could not get terminal size");
        let lines = sources[source_index].lines.lock().expect("Could not take lock in term_thread");
        if lines.len() < rows as usize { // If there aren't many lines we can start in autoscroll
            pos_by_source[source_index] = None;
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
                                let lines = sources[source_index].lines.lock().expect("Could not take lock in ArrrowUp event handler");
                                page_by(&lines, &mut pos_by_source[source_index], -1);
                            }
                        }
                        crossterm::event::KeyCode::Char('u') | crossterm::event::KeyCode::Char('U') | crossterm::event::KeyCode::PageUp => {
                            {
                                let lines = sources[source_index].lines.lock().expect("Could not take lock in PgUp event handler");
                                page_by(&lines, &mut pos_by_source[source_index], -(PAGE_UP_SIZE as i32));
                            }
                        }
                        crossterm::event::KeyCode::Down => {
                            {
                                let lines = sources[source_index].lines.lock().expect("Could not take lock in ArrowDown event handler");
                                page_by(&lines, &mut pos_by_source[source_index], 1);
                            }
                        }
                        crossterm::event::KeyCode::Char('d') | crossterm::event::KeyCode::Char('D') | crossterm::event::KeyCode::PageDown | crossterm::event::KeyCode::Char(' ') => {
                            {
                                let lines = sources[source_index].lines.lock().expect("Could not take lock in PgDn event handler");
                                page_by(&lines, &mut pos_by_source[source_index], PAGE_UP_SIZE as i32);
                            }
                        }
                        crossterm::event::KeyCode::Enter => {
                            pos_by_source[source_index] = None;
                            {
                                let lines = sources[source_index].lines.lock().expect("Could not take lock in Enter event handler");
                                overwrite_last_n_lines(&lines, pos_by_source[source_index], None);
                            }
                        }
                        crossterm::event::KeyCode::Char('g') | crossterm::event::KeyCode::Char('G') => {
                            let mut highlight_line_no = None;
                            {
                                let lines = sources[source_index].lines.lock().expect("Could not take lock in goto line event handler");
                                if event.modifiers.contains(KeyModifiers::SHIFT) {
                                    pos_by_source[source_index] = None;
                                } else {
                                    let line_no: Option<usize> = handle_go_to_line(pos_by_source[source_index], lines.len(), &term_rx);
                                    highlight_line_no = line_no;
                                    pos_by_source[source_index] = pos_with_in_view(line_no, PAGE_UP_SIZE);
                                }
                                overwrite_last_n_lines(&lines, pos_by_source[source_index], highlight_line_no);
                            }
                        }
                        crossterm::event::KeyCode::Char('/') => {
                            handle_search_mode(&mut pos_by_source[source_index], &sources[source_index].lines, &term_rx, PAGE_UP_SIZE, false);
                        }
                        crossterm::event::KeyCode::Char('r') | crossterm::event::KeyCode::Char('R') => {
                            handle_search_mode(&mut pos_by_source[source_index], &sources[source_index].lines, &term_rx, PAGE_UP_SIZE, true);
                        },
                        crossterm::event::KeyCode::Char('s') | crossterm::event::KeyCode::Char('S') => {
                            source_index += 1;
                            source_index %= sources.len();

                            {
                                let lines = sources[source_index].lines.lock().expect("Could not take lock in source switch event handler");
                                overwrite_last_n_lines(&lines, pos_by_source[source_index], None);
                                write_status_message(format!("Switched to source: {}", sources[source_index].name).as_str());
                            }
                        }
                        _ => {}
                    }
                },
                TerminalThreadMessage::Resize(_, _) => {
                    let lines = sources[source_index].lines.lock().expect("Could not take lock in resize event handler");
                    overwrite_last_n_lines(&lines, pos_by_source[source_index], None);
                }
                TerminalThreadMessage::Read => {
                    let lines = sources[source_index].lines.lock().expect("Could not take lock in read event handler");
                    overwrite_last_n_lines(&lines, pos_by_source[source_index], None);
                }
            }
        }
    }

    execute!(stdout(), EnableLineWrap).unwrap();
    execute!(stdout(), LeaveAlternateScreen).unwrap();
    disable_raw_mode().expect("Could not exit raw mode");

    // This will bring all of our threads down with us
    exit(0);
}


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

fn jump_to_match(lines: &Vec<String>, matches: &Vec<usize>, pos: &mut Option<usize>, page_up_size: usize, match_no: usize) -> Result<(), ()> {
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
fn handle_search_mode(pos: &mut Option<usize>, lines_mtx: &Mutex<Vec<String>>, term_rx: &mpsc::Receiver<TerminalThreadMessage>, page_up_size: usize, match_regex: bool) {
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
        let _ = jump_to_match(&lines, &matches, pos, page_up_size, 0);
        write_status_message(&format!("{}: {}", prompt, search));
    }

    {
        let mut match_no = 0;
        let matches = get_matches(&lines, search.trim(), match_regex);
        let _ = jump_to_match(&lines, &matches, pos, page_up_size, match_no);

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
                            let _ = jump_to_match(&lines, &matches, pos, page_up_size, match_no);
                        }
                        crossterm::event::KeyCode::Char('p') | crossterm::event::KeyCode::Up |  crossterm::event::KeyCode::Left => {
                            match_no = if match_no > 0 { match_no - 1 } else { matches.len() - 1 };
                            let _ = jump_to_match(&lines, &matches, pos, page_up_size, match_no);
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
    if line_no > n_lines {
        None
    } else if line_no == 0 {
        Some(0)
    } else {
        Some(line_no - 1)
    }
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
