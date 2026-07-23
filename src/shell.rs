use nix::unistd::Pid;

// You may assume that these constants are large enough.
const MAXLINE: usize = 1024;	  // max line size
const MAXARGS: usize = 128;	  // max args on a command line
const MAXJOBS: usize = 16;	  // max jobs at any point in time
const MAXJID: usize = 1 << 16; // max job ID

// The job states are:
#[derive(Debug)]
enum JobState {
    UNDEF, // undefined
    FG,    // running in foreground
    BG,    // running in background
    ST,    // stopped
}

#[derive(Debug)]
struct Job {
    pid : Option<Pid>,	    // job PID
	jid: usize,             // job ID [1, 2, ...]
	state: JobState,        // UNDEF, FG, BG, or ST
	cmdline: String,        // command line
}

struct Shell {
    jobs: Vec<Job>,
    verbose_flag: bool,
    prompt_flag: bool,
}

fn unix_error(msg: &str) {
    println!("{}: {}", msg, nix::errno::Errno::last());
    std::process::exit(1);
}

