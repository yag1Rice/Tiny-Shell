use errno::{errno, set_errno};
use nix::sys::signal::{SigSet, SigmaskHow, Signal, kill, pthread_sigmask};
use nix::{libc, unistd::Pid};

// You may assume that these constants are large enough.
const MAXLINE: usize = 1024; // max line size
const MAXARGS: usize = 128; // max args on a command line
const MAXJOBS: usize = 16; // max jobs at any point in time
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
    pid: Option<Pid>, // job PID
    jid: usize,       // job ID [1, 2, ...]
    state: JobState,  // UNDEF, FG, BG, or ST
    cmdline: String,  // command line
}

pub struct Shell {
    jobs: Vec<Job>,
    verbose_flag: bool,
    prompt_flag: bool,
    next_jid: usize, // next job ID to allocate
}

impl Shell {
    pub fn new(prompt_flag: bool, verbose_flag: bool) -> Self {
        Self {
            jobs: Self::initjobs(),
            verbose_flag,
            prompt_flag,
            next_jid: 1,
        }
    }

    fn initjobs() -> Vec<Job> {
        let mut jobs: Vec<Job> = Vec::new();
        for _ in 0..MAXJOBS {
            let job = Job {
                pid: None,
                jid: 0,
                state: JobState::UNDEF,
                cmdline: String::new(),
            };
            jobs.push(job);
        }
        jobs
    }

    pub fn initpath(&self, pathstr: &str) -> Vec<String> {
        let mut pathvec: Vec<String> = Vec::new();

        if pathstr.is_empty() {
            return pathvec;
        }

        for path in pathstr.split(':') {
            pathvec.push(path.to_string());
        }

        pathvec
    }

    pub fn builtin_cmd(&self, argv: &[String]) -> bool {
        match argv[0].as_str() {
            "quit" => {
                std::process::exit(0);
            }
            "jobs" => {
                self.listjobs();
                true
            }
            "bg" | "fg" => {
                self.do_bgfg(argv);
                true
            }
            _ => false,
        }
    }

    fn listjobs(&self) {
        for job in &self.jobs {
            if let Some(pid) = job.pid {
                let state_str = match job.state {
                    JobState::BG => "Running",
                    JobState::FG => "Foreground",
                    JobState::ST => "Stopped",
                    JobState::UNDEF => "listjobs: Internal error",
                };

                println!("[{}] ({}) {} {}", job.jid, pid, state_str, job.cmdline);
            }
        }
    }

    fn eval(&self) {
        // TODO: Implementation for evaluating a command line
    }

    fn parseline(&self) -> () {
        // TODO: Implementation for parsing command line
    }

    fn do_bgfg(&self, argv: &[String]) -> () {
        // TODO: Implementation for bg and fg commands
    }

    fn waitfg(&self, pid: Pid) {
        let mut mask = SigSet::empty();
        let mut prev = SigSet::empty();

        mask.add(Signal::SIGCHLD);

        if let Err(e) = pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&mask), Some(&mut prev)) {
            unix_error(&format!("pthread_sigmask error: {}", e));
        }

        while self.fgpid() == Some(pid) {
            let _ = prev.suspend();
        }

        if let Err(e) = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&prev), None) {
            unix_error(&format!("pthread_sigmask error: {}", e));
        }
    }

    fn fgpid(&self) -> Option<Pid> {
        for job in &self.jobs {
            if let JobState::FG = job.state {
                return job.pid;
            }
        }
        None
    }

    fn pid2jid(&self, pid: Pid) -> usize {
        if pid.as_raw() < 1 {
            return 0;
        }
        for job in &self.jobs {
            if let Some(job_pid) = job.pid {
                if job_pid == pid {
                    return job.jid;
                }
            }
        }
        0
    }

    fn getjobpid(&self, pid: Pid) -> Option<&Job> {
        if pid.as_raw() < 1 {
            return None;
        }

        for job in &self.jobs {
            if let Some(job_pid) = job.pid {
                if job_pid == pid {
                    return Some(job);
                }
            }
        }
        None
    }

    fn getjobjid(&self, jid: usize) -> Option<&Job> {
        if jid < 1 {
            return None;
        }

        for job in &self.jobs {
            if job.jid == jid {
                return Some(job);
            }
        }
        None
    }

    fn addjob(&mut self, pid: Pid, state: JobState, cmdline: &str) -> bool {
        if pid.as_raw() < 1 {
            return false;
        }

        for job in &mut self.jobs {
            if job.pid.is_none() {
                job.pid = Some(pid);
                job.state = state;
                job.jid = self.next_jid;
                job.cmdline = cmdline.to_string();

                self.next_jid += 1;
                if self.next_jid > MAXJID {
                    self.next_jid = 1;
                }
                if self.verbose_flag {
                    println!("Added job [{}] {} {}", job.jid, pid, cmdline);
                }
                return true;
            }
        }

        println!("Tried to create too many jobs");
        false
    }

    fn deletejob(&mut self, pid: Pid) -> bool {
        if pid.as_raw() < 1 {
            return false;
        }

        for job in &mut self.jobs {
            if let Some(job_pid) = job.pid {
                if job_pid == pid {
                    job.pid = None;
                    job.jid = 0;
                    job.state = JobState::UNDEF;
                    job.cmdline.clear();
                    if self.verbose_flag {
                        println!("Deleted job [{}] {}", job.jid, pid);
                    }
                    return true;
                }
            }
        }

        false
    }

    fn signal_handler(&self, signal: Signal) -> () {
        let olderrrno = errno();
        if let Some(pid) = self.fgpid() {
            let process_group_id = pid.as_raw();
            if let Err(e) = kill(Pid::from_raw(-process_group_id), signal) {
                sio_puts(&format!("kill error: {}\n", e));
            }
            set_errno(olderrrno);
        }
    }

    pub fn sigint_handler(&self) -> () {
        self.signal_handler(Signal::SIGINT)
    }

    pub fn sigtstp_handler(&self) -> () {
        self.signal_handler(Signal::SIGTSTP)
    }

    pub fn sigquit_handler(&self) -> () {
        sio_puts("Terminating after receipt of SIGQUIT signal\n");
        std::process::exit(1);
    }

    pub fn sigchld_handler(&self) -> () {
        // TODO: Implementation for handling SIGCHLD signal
    }
}

fn unix_error(msg: &str) -> () {
    println!("{}: {}", msg, errno());
    std::process::exit(1);
}

fn sio_puts(s: &str) -> () {
    let msg = s.as_bytes();
    unsafe {
        libc::write(libc::STDOUT_FILENO, msg.as_ptr() as *const _, msg.len());
    }
}
