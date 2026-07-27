use std::env;

use errno::{errno, set_errno};
use nix::errno::Errno;
use nix::libc;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, sigaction};
use nix::sys::signal::{SigSet, SigmaskHow, Signal, kill, pthread_sigmask};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, execve, fork, setpgid};
use signal_hook::consts::{SIGCHLD, SIGINT, SIGQUIT, SIGTSTP};
use signal_hook::iterator::Signals;

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

    pub fn builtin_cmd(&mut self, argv: &[String], signals: &mut Signals) -> bool {
        match argv[0].as_str() {
            "quit" => {
                std::process::exit(0);
            }
            "jobs" => {
                self.listjobs();
                true
            }
            "bg" | "fg" => {
                self.do_bgfg(argv, signals);
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

                print!("[{}] ({}) {} {}", job.jid, pid, state_str, job.cmdline);
            }
        }
    }

    pub fn eval(&mut self, cmdline: &str, pathvec: &[String], signals: &mut Signals) {
        let (argv, isbg) = self.parseline(cmdline);

        if argv.is_empty() {
            return;
        }

        let mut mask = SigSet::empty();
        let mut prev = SigSet::empty();

        mask.add(Signal::SIGCHLD);

        if !self.builtin_cmd(&argv, signals) {
            if let Err(e) = pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&mask), Some(&mut prev)) {
                unix_error(&format!("pthread_sigmask error: {}", e));
            }

            match unsafe { fork() } {
                Ok(ForkResult::Parent { child, .. }) => {
                    if let Err(e) = setpgid(child, child) {
                        unix_error(&format!("setpgid error: {}", e));
                    }
                    let state = if isbg { JobState::BG } else { JobState::FG };
                    self.addjob(child, state, cmdline);

                    if isbg {
                        println!("[{}] ({}) {}", self.pid2jid(child), child, cmdline.trim());
                    }

                    if let Err(e) = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&prev), None) {
                        unix_error(&format!("pthread_sigmask error: {}", e));
                    }

                    if !isbg {
                        self.waitfg(child, signals);
                    }
                }
                Ok(ForkResult::Child) => {
                    let dfl = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
                    for sig in [
                        Signal::SIGINT,
                        Signal::SIGTSTP,
                        Signal::SIGQUIT,
                        Signal::SIGCHLD,
                    ] {
                        unsafe {
                            let _ = sigaction(sig, &dfl);
                        }
                    }

                    if let Err(e) = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&prev), None) {
                        unix_error(&format!("pthread_sigmask error: {}", e));
                    }
                    if let Err(e) = setpgid(Pid::from_raw(0), Pid::from_raw(0)) {
                        unix_error(&format!("setpgid error: {}", e));
                    }

                    let args = argv
                        .iter()
                        .map(|arg| {
                            std::ffi::CString::new(arg.as_str())
                                .expect("Should be able to create a CString from argument")
                        })
                        .collect::<Vec<_>>();
                    let env = env::vars()
                        .map(|(key, value)| {
                            std::ffi::CString::new(format!("{}={}", key, value)).expect(
                                "Should be able to create a CString from environment variable",
                            )
                        })
                        .collect::<Vec<_>>();

                    if !argv[0].contains('/') && pathvec.len() > 0 {
                        for path in pathvec {
                            let full_path: String;

                            if path.is_empty() {
                                full_path = format!("./{}", argv[0]);
                            } else {
                                full_path = format!("{}/{}", path, argv[0]);
                            }

                            let cmd = std::ffi::CString::new(full_path.as_str())
                                .expect("Should be able to create a CString from full path");

                            let _ = execve(&cmd, &args, &env);
                        }
                    } else {
                        let cmd = std::ffi::CString::new(argv[0].as_str())
                            .expect("Should be able to create a CString from first argument");

                        let _ = execve(&cmd, &args, &env);
                        println!("{}: Command not found", argv[0]);
                        std::process::exit(0);
                    }

                    println!("{}: Command not found", argv[0]);
                    std::process::exit(0);
                }
                Err(_) => {
                    unix_error("fork error");
                }
            }
        }
    }

    pub fn parseline(&self, cmdline: &str) -> (Vec<String>, bool) {
        let mut argv = Vec::new();

        let mut trimmed = cmdline.trim();

        if trimmed.is_empty() {
            return (argv, false);
        }

        while !trimmed.is_empty() {
            // Handle quoted arguments
            if trimmed.starts_with('\'') {
                let end = trimmed[1..].find('\'').unwrap_or(trimmed.len() - 1);
                argv.push(trimmed[1..1 + end].to_string());
                trimmed = &trimmed[1 + end + 1..];
            }
            // Handle unquoted arguments
            else {
                let end = trimmed.find(' ').unwrap_or(trimmed.len());
                argv.push(trimmed[..end].to_string());
                trimmed = &trimmed[end..];
            }

            // Skip whitespace
            trimmed = trimmed.trim_start();
        }

        let mut bg = false;

        if let Some(last) = argv.last() {
            if last == "&" {
                argv.pop();
                bg = true;
            }
        }

        (argv, bg)
    }

    fn do_bgfg(&mut self, argv: &[String], signals: &mut Signals) -> () {
        if argv.len() < 2 {
            println!("{} command requires PID or %jobid argument", argv[0]);
            return;
        }

        if argv[1].starts_with('%') && argv[1][1..].parse::<usize>().is_ok() {
            let jid = argv[1][1..]
                .parse::<usize>()
                .expect("There should be a JID for this job!");
            let job = self.getjobjid(jid);
            let job = match job {
                Some(job) => job,
                None => {
                    println!("{}: No such job", argv[1]);
                    return;
                }
            };

            if argv[0] == "bg" {
                if let Err(e) = kill(
                    Pid::from_raw(
                        -(job
                            .pid
                            .expect("There should be a PID for this job!")
                            .as_raw()),
                    ),
                    Signal::SIGCONT,
                ) {
                    sio_puts(&format!("kill error: {}\n", e));
                }
                job.state = JobState::BG;
                print!(
                    "[{}] ({}) {}",
                    job.jid,
                    job.pid.expect("There should be a PID for this job"),
                    job.cmdline
                );
            } else {
                if let Err(e) = kill(
                    Pid::from_raw(
                        -(job
                            .pid
                            .expect("There should be a PID for this job!")
                            .as_raw()),
                    ),
                    Signal::SIGCONT,
                ) {
                    sio_puts(&format!("kill error: {}\n", e));
                }
                job.state = JobState::FG;
                let pid = job.pid.expect("There should be a PID for this job!");
                self.waitfg(pid, signals);
            }
        } else if argv[1].chars().all(|c| c.is_ascii_digit()) {
            let pid = Pid::from_raw(argv[1].parse::<i32>().unwrap());
            let job = self.getjobpid(pid);
            let job = match job {
                Some(job) => job,
                None => {
                    println!("({}): No such process", pid);
                    return;
                }
            };

            if argv[0] == "bg" {
                if let Err(e) = kill(Pid::from_raw(-pid.as_raw()), Signal::SIGCONT) {
                    sio_puts(&format!("kill error: {}\n", e));
                }
                job.state = JobState::BG;
                print!("[{}] ({}) {}", job.jid, pid, job.cmdline);
            } else {
                if let Err(e) = kill(Pid::from_raw(-pid.as_raw()), Signal::SIGCONT) {
                    sio_puts(&format!("kill error: {}\n", e));
                }
                job.state = JobState::FG;
                self.waitfg(pid, signals);
            }
        } else {
            print!("{}: argument must be a PID or %jobid\n", argv[0]);
        }
    }

    fn waitfg(&mut self, pid: Pid, signals: &mut Signals) {
        while self.fgpid() == Some(pid) {
            for signal in signals.wait() {
                match signal {
                    SIGCHLD => self.sigchld_handler(),
                    SIGINT => self.sigint_handler(),
                    SIGTSTP => self.sigtstp_handler(),
                    SIGQUIT => self.sigquit_handler(),
                    _ => {}
                }
                if self.fgpid() != Some(pid) {
                    return;
                }
            }
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

    fn getjobpid(&mut self, pid: Pid) -> Option<&mut Job> {
        if pid.as_raw() < 1 {
            return None;
        }

        for job in &mut self.jobs {
            if let Some(job_pid) = job.pid {
                if job_pid == pid {
                    return Some(job);
                }
            }
        }
        None
    }

    fn getjobjid(&mut self, jid: usize) -> Option<&mut Job> {
        if jid < 1 {
            return None;
        }

        for job in &mut self.jobs {
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
                    if self.verbose_flag {
                        println!("Deleted job [{}] {}", job.jid, pid);
                    }
                    job.pid = None;
                    job.jid = 0;
                    job.state = JobState::UNDEF;
                    job.cmdline.clear();
                    self.next_jid = self.maxjid() + 1;
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

    pub fn sigchld_handler(&mut self) -> () {
        let olderrrno = errno();

        loop {
            match waitpid(
                Pid::from_raw(-1),
                Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED),
            ) {
                Ok(WaitStatus::Exited(pid, _)) if self.getjobpid(pid).is_some() => {
                    self.deletejob(pid);
                }
                Ok(WaitStatus::Signaled(pid, _, _)) if let Some(job) = self.getjobpid(pid) => {
                    sio_puts(&format!(
                        "Job [{}] ({}) terminated by signal SIGINT\n",
                        job.jid, pid
                    ));
                    self.deletejob(pid);
                }
                Ok(WaitStatus::Stopped(pid, _)) if let Some(job) = self.getjobpid(pid) => {
                    job.state = JobState::ST;
                    sio_puts(&format!(
                        "Job [{}] ({}) stopped by signal SIGTSTP\n",
                        job.jid, pid
                    ));
                }
                Ok(_) => break,              // No changes in child processes
                Err(Errno::ECHILD) => break, // No more child processes
                Err(_) => {
                    sio_puts("waitpid error");
                    break;
                }
            }
        }

        set_errno(olderrrno);
    }

    fn maxjid(&self) -> usize {
        let mut max = 0;
        for job in &self.jobs {
            if job.jid > max {
                max = job.jid;
            }
        }
        max
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
