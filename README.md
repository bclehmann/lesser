# lesser

This is a basic terminal pager that has support for searching. I made this because I found myself reaching for a better way to read through log files at work. And secondarily because I want to work more with rust.

## Features

- Line-by-line paging with up/down arrows
- Search by pressing `/`, or regex search with `r`
- Suitable for paging through streaming input
    - note that on Powershell `foo | lesser` will _not_ pipe anything in until `foo` terminates. I may make a workaround, but even though I'm a Windows user I use WSL for work, so I haven't found a way to address this
- Open multiple files at once: `lesser file1 file2 ...` or with a glob like `lesser dir/*` (switch between them with `s`)
- Open a file in watch mode with `--watch`, this will subscribe to updates


## Controls

- Quit: `q`, `ESC`
- Line up/down: arrow keys
- Page up/down: `u` and `d` or `PgUp` and `PgDn`
- Go to end: `Enter`
- Enter search mode: `/`
   - Next/prev occurrences with arrow keys
   - Exit search mode: Escape
- Enter search mode (regex): `r`
- Go to line: `g`
    - Enter line number and press `Enter`, or press `g` again to go to start
- Go to next file: `s`

## Code

I wrote this in an afternoon and I haven't really cleaned up the code since I got it to the level I needed. But it'll likely need a little spring cleaning before non-trivial changes will make much sense. It's all GPL-ed, so feel free to hack in your own usecase.

## Features I'd like to add

- Rewriting the whole screen on every change is probably not ideal from a performance perspective. But I don't yet know if it'll be a big problem
- Multiplexing and merging different files
  - My thinking is `lesser logs/*` should be able to merge all the logs together in timestamp order (likely with a prefix denoting which file a line belongs to), not just let you switch between them
