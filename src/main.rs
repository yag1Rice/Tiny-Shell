mod shell;
use shell::Shell;

use clap::{CommandFactory, Parser};
use signal_hook::consts::{SIGCHLD, SIGINT, SIGQUIT, SIGTSTP};
use signal_hook::iterator::Signals;
#[derive(Parser)]
#[command(name = "shell", about = "Tiny Shell", override_usage = "shell [-hvp]")]
struct Args {
    #[arg(short, help = "do not emit a command prompt")]
    prompt_flag: bool, // Flag to indicate to not display the prompt

    #[arg(short, help = "print additional diagnostic information")]
    verbose_flag: bool, // Flag to indicate whether to display verbose error messages
}

fn main() {
    // if std::env::args().len() == 1 {
    //     Args::command().print_help().unwrap();
    //     return;
    // }

    let args = Args::parse(); // Parse command-line arguments

    let mut shell = Shell::new(args.prompt_flag, args.verbose_flag); // Create a new Shell instance

    let mut signals =
        Signals::new([SIGCHLD, SIGINT, SIGTSTP, SIGQUIT]).expect("Failed to create signal handler");

    let path = std::env::var("PATH");
    let path = match path {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error: PATH environment variable not found.");
            std::process::exit(1);
        }
    };

    let pathvec = shell.initpath(&path);

    loop {
        if !(args.prompt_flag) {
            print!("tsh> ");
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }

        for signal in signals.pending() {
            match signal {
                SIGCHLD => shell.sigchld_handler(),
                SIGINT => shell.sigint_handler(),
                SIGTSTP => shell.sigtstp_handler(),
                SIGQUIT => shell.sigquit_handler(),
                _ => {}
            }
        }

        let mut cmdline = String::new();

        match std::io::stdin().read_line(&mut cmdline) {
            Ok(0) => {
                std::process::exit(0);
            }
            Ok(_) => {
                shell.eval(&cmdline, &pathvec, &mut signals);
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
            }
            Err(_) => {
                eprintln!("stdin read_line error");
                std::process::exit(1);
            }
        }
    }
}
