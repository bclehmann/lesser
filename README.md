# lesser

This is a basic terminal pager that has support for searching. I made this because I found myself reaching for a better way to read through log files at work. And secondarily because I want to work more with rust.

## Features

- Line-by-line paging with up/down arrows
- Search by pressing `/`
- Suitable for paging through streaming input
    - note that on Powershell `foo | lesser` will _not_ pipe anything in until `foo` terminates. I may make a workaround, but even though I'm a Windows user I use WSL for work, so I haven't found a way to address this

## Controls

- Quit: `q`, `ESC`
- Line up: `u`, Up arrow
- Line down: `d`, Down arrow, space
- Go to end: `Enter`
- Enter search mode: `/`
   - Next occurrence: `Enter`
   - Exit search mode: Any other key

## Code

I wrote this in an afternoon and I haven't really cleaned up the code since I got it to the level I needed. But it'll likely need a little spring cleaning before non-trivial changes will make much sense. It's all GPL-ed, so feel free to hack in your own usecase.

## Features I'd like to add

- Paging one line at a time is a little slow, this should probably support PgUp/PgDn
- Polling is maybe not ideal. It's already multithreaded with a reader thread and a terminal thread, so it might make sense to split input handling into a separate thread, so we don't need to switch between waiting for keyboard events and waiting for activity on STDIN
- Support opening files, rather than just using STDIN. This could probably address the Powershell issue above
- Regex search
- Rewriting the whole screen on every change is probably not ideal from a performance perspective. But I don't yet know if it'll be a big problem

