# Tiny Shell (`tsh`)

A minimal, POSIX-style job-control shell written in Rust. It forks and execs external programs,
resolves commands against `PATH`, supports foreground/background execution, and implements
signal-driven job control (`SIGINT`, `SIGTSTP`, `SIGCHLD`, `SIGQUIT`).

This is a Rust re-implementation of the classic "tiny shell" systems-programming exercise.

## Features

- Command execution via `fork(2)` + `execve(2)`
- `PATH` resolution (falls back to the literal path when the command contains `/`)
- Background execution with a trailing `&`
- Single-quoted argument grouping (`echo 'hello world'`)
- Job table with job IDs (`%1`) and PIDs
- Each job runs in its own process group so signals hit the whole group
- Signal handling for `SIGINT` (Ctrl-C), `SIGTSTP` (Ctrl-Z), `SIGCHLD`, and `SIGQUIT`
- `SIGCHLD` blocked around `fork`/`addjob` to avoid the classic add/delete race

## Built-in Commands

| Command | Description |
| --- | --- |
| `quit` | Exit the shell immediately |
| `jobs` | List all current jobs and their states |
| `bg <pid \| %jid>` | Resume a stopped job in the background |
| `fg <pid \| %jid>` | Resume a job in the foreground and wait for it |

Anything else is treated as an external program.

## Requirements

- A recent stable Rust toolchain (the code uses `if let` guards in `match`, so Rust **1.91+**)
- A Unix-like OS (macOS or Linux) — this shell relies on POSIX process and signal APIs

## Dependencies

| Crate | Purpose |
| --- | --- |
| [`clap`](https://crates.io/crates/clap) | Command-line argument parsing (derive API) |
| [`nix`](https://crates.io/crates/nix) | Safe bindings for `fork`, `execve`, `waitpid`, `setpgid`, `kill`, sigmasks |
| [`signal-hook`](https://crates.io/crates/signal-hook) | Iterator-based, async-signal-safe signal delivery |
| [`errno`](https://crates.io/crates/errno) | Save/restore `errno` inside signal handling code |

## Building

```sh
cargo build
```

Release build:

```sh
cargo build --release
```

## Running

```sh
cargo run
```

or, after building:

```sh
./target/debug/shell
```

### CLI Options

```
Usage: shell [-hvp]

Options:
  -p           Do not emit a command prompt (useful for automated testing)
  -v           Print additional diagnostic information
  -h, --help   Print help
```

## Usage Examples

```
tsh> /bin/echo hello
hello

tsh> echo 'hello world'
hello world

tsh> sleep 30 &
[1] (48213) sleep 30 &

tsh> jobs
[1] (48213) Running sleep 30 &

tsh> fg %1
^Z
Job [1] (48213) stopped by signal SIGTSTP

tsh> bg %1
[1] (48213) sleep 30 &

tsh> quit
```

## Project Layout

```
shell/
├── Cargo.toml
├── README.md
└── src/
    ├── main.rs    # CLI parsing, signal registration, REPL loop
    └── shell.rs   # Shell state, job table, eval/parseline, builtins, signal handlers
```

### `src/main.rs`

Parses flags with `clap`, creates the `Shell`, registers the signal set with `signal_hook`,
splits `PATH` into a search vector, then loops: print prompt → drain pending signals →
read a line → `shell.eval(...)`. EOF (`Ctrl-D`) exits with status `0`.

### `src/shell.rs`

| Item | Role |
| --- | --- |
| `Shell` | Owns the job table, flags, and the next job ID |
| `Job` / `JobState` | Job record (`pid`, `jid`, `state`, `cmdline`) and its state (`UNDEF`/`FG`/`BG`/`ST`) |
| `parseline` | Tokenizes the command line, handles single quotes, detects trailing `&` |
| `eval` | Blocks `SIGCHLD`, forks, sets the child's process group, adds the job, waits if foreground |
| `builtin_cmd` / `do_bgfg` | Built-in dispatch and `bg`/`fg` implementation |
| `waitfg` | Blocks on signals until the foreground job leaves the foreground |
| `sigchld_handler` | Reaps children with `WNOHANG \| WUNTRACED`, updates/deletes jobs |
| `sigint_handler` / `sigtstp_handler` | Forward the signal to the foreground process group |
| `sigquit_handler` | Terminates the shell |
| `sio_puts` | Async-signal-safe output via raw `write(2)` |

## Design Notes

- **Process groups.** `setpgid` is called in both the parent and child (a standard race-avoidance
  idiom) so the child always lands in its own group before signals are forwarded.
- **Signal safety.** Handlers save and restore `errno` and use `write(2)` directly rather than
  Rust's buffered `println!`, which is not async-signal-safe.
- **Signal delivery model.** Instead of installing raw handlers, signals are queued by
  `signal-hook` and drained from the main loop and from `waitfg`, which keeps mutable access to
  the job table safe under Rust's borrow rules.
- **Fixed-size job table.** `MAXJOBS` slots are pre-allocated; empty slots have `pid == None`.

## Limitations

- No pipes (`|`), I/O redirection (`>`, `<`), or command substitution
- No double-quote or backslash escaping (single quotes only)
- No globbing, environment-variable expansion, or shell scripting constructs
- No `cd` built-in; the shell's working directory is fixed
- Job IDs are recomputed from the max on deletion rather than tracked in a free list