mod shell;
use shell::Shell;

use clap::{CommandFactory, Parser};

#[derive(Parser)]
#[command(name = "shell", about = "Tiny Shell", override_usage = "shell [-hvp]")]
struct Args {
    #[arg(short, help = "do not emit a command prompt")]
    prompt_flag: bool, // Flag to indicate to not display the prompt

    #[arg(short, help = "print additional diagnostic information")]
    verbose_flag: bool, // Flag to indicate whether to display verbose error messages
}

fn main() {
    if std::env::args().len() == 1 {
        Args::command().print_help().unwrap();
        return;
    }

    let args = Args::parse(); // Parse command-line arguments

    let mut shell = Shell::new(args.prompt_flag, args.verbose_flag); // Create a new Shell instance
}
