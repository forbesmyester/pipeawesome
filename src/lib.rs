use std::thread::JoinHandle;
use std::collections::HashMap;
use std::io::Write;
use core::ops::Range;
use std::sync::mpsc::channel;
use std::path::Path;
use std::ffi::OsStr;
use std::sync::mpsc::{sync_channel, Sender, SyncSender, TrySendError, TryRecvError, Receiver};

type Line = Option<String>;


type ReadFrom = HashMap<usize, usize>;
type WroteTo = HashMap<usize, usize>;
#[derive(Debug)]
pub enum StoppedBy {
    IOError(std::io::Error),
    StillWorking,
    ExhaustedInput,
    Waiting,
    OutputFull,
    InternallyFull,
}


#[derive(Debug)]
pub struct ProcessStatus {
    pub read_from: ReadFrom,
    pub wrote_to: WroteTo,
    pub stopped_by: StoppedBy
}


impl ProcessStatus {

    fn new() -> ProcessStatus {
        ProcessStatus {
            read_from: HashMap::new(),
            wrote_to: HashMap::new(),
            stopped_by: StoppedBy::StillWorking,
        }
    }

    fn add_to_hm<I>(hm: &mut HashMap<usize, usize>, r: I) where I: Iterator<Item = usize> {
        for i in r {
            match hm.get_mut(&i) {
                Some(entry) => *entry = *entry + 1,
                None => {
                    hm.insert(i, 1);
                }
            }
        }
    }


    fn add_to_wrote_to<I>(&mut self, r: I) where I: Iterator<Item = usize> {
        ProcessStatus::add_to_hm(&mut self.wrote_to, r);
    }

    fn add_to_read_from<I>(&mut self, r: I) where I: Iterator<Item = usize> {
        ProcessStatus::add_to_hm(&mut self.read_from, r);
    }

    fn set_stopped_by(&mut self, sb: StoppedBy) {
        std::mem::replace(&mut self.stopped_by, sb);
    }
}


#[derive(Debug)]
pub enum SinkWriteError {
    TryRecvError(TryRecvError),
    WriteError(std::io::Error),
}

impl std::fmt::Display for SinkWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SinkWriteError::WriteError(e) => write!(f, "SinkWriteError: WriteError: {}", e),
            SinkWriteError::TryRecvError(e) => write!(f, "SinkWriteError: TryRecvError: {}", e),
        }
    }
}


#[derive(Debug)]
pub enum OutputAlreadyUsedError {
    OutputAlreadyUsedError,
}

impl std::fmt::Display for OutputAlreadyUsedError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            OutputAlreadyUsedError::OutputAlreadyUsedError => write!(f, "OutputAlreadyUsedError"),
        }
    }
}


#[derive(Debug)]
pub enum InputAlreadyUsedError {
    InputAlreadyUsedError,
}

impl std::fmt::Display for InputAlreadyUsedError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            InputAlreadyUsedError::InputAlreadyUsedError => write!(f, "InputAlreadyUsedError"),
        }
    }
}


#[derive(Debug)]
#[derive(PartialEq)]
pub enum SinkWriteStatus {
    FINISHED,
    ONGOING,
}

fn write<W>(i: &Receiver<Line>, w: &mut W) -> Result<SinkWriteStatus, SinkWriteError>
    where W: std::io::Write
{

    match i.try_recv() {
        Ok(None) => {
            match w.flush() {
                Ok(()) => Ok(SinkWriteStatus::FINISHED),
                Err(e) => Err(SinkWriteError::WriteError(e)),
            }
        },
        Ok(Some(line)) => {
            w.write_all(line.as_bytes())
                .map(|_x| SinkWriteStatus::ONGOING)
                .map_err(|e| SinkWriteError::WriteError(e))
        },
        Err(x) => {
            Err(SinkWriteError::TryRecvError(x))
        }
    }

}


fn do_sync_send(tx: &SyncSender<Line>, msg: &Option<String>) {
    loop {
        match tx.send(msg.clone()) {
            Ok(_) => return (),
            Err(_) => (),
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}


pub struct Tap<R> where R: std::io::Read {
    tx: Option<SyncSender<Line>>,
    get_buf: Option<fn() -> R>,
}

impl <R: std::io::Read> Tap<R> where {

    pub fn new(get_buf: fn() -> R) -> Tap<R> {
        Tap {
            tx: None,
            get_buf: Some(get_buf),
        }
    }

    pub fn get_output(&mut self) -> Result<Receiver<Line>, OutputAlreadyUsedError> {

        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);

        if self.tx.is_none() {
            std::mem::replace(&mut self.tx, Some(tx));
            return Ok(rx)
        }
        Err(OutputAlreadyUsedError::OutputAlreadyUsedError)
    }

}


impl <R: 'static +  std::io::Read> GetProcessable<TapProcessor<R>> for Tap<R> {
    fn get_processor(&mut self) -> Result<TapProcessor<R>, ProcessorAlreadyUsedError> {

        match (std::mem::take(&mut self.tx), std::mem::take(&mut self.get_buf)) {
            (Some(tx), Some(get_buf)) => {
                Ok(TapProcessor::new(get_buf, tx))
            },
            _ => Err(ProcessorAlreadyUsedError::ProcessorAlreadyUsedError)
        }

    }
}

#[derive(Debug)]
pub struct TapProcessor<R> {
    tx: SyncSender<Line>,
    int_tx: Option<SyncSender<Line>>,
    int_rx: Receiver<Line>,
    jh: Option<JoinHandle<()>>,
    get_buf: Option<fn() -> R>,
    pending: Option<Line>,
}

impl <R: 'static +  std::io::Read> TapProcessor<R> {

    fn new(get_buf: fn() -> R, tx: SyncSender<Line>) -> TapProcessor<R> {
        let (int_tx, int_rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
        let mut t = TapProcessor {
            get_buf: Some(get_buf),
            tx: tx,
            pending: None,
            jh: None,
            int_tx: Some(int_tx),
            int_rx,
        };
        t.setup();
        t
    }

    fn setup(&mut self) {

        fn command_read_std<R>(br: &mut R, tx: &SyncSender<Line>) -> std::io::Result<bool>
            where R: std::io::BufRead
        {
            let mut s = String::new();
            let count = br.read_line(&mut s)?;
            let v = if count == 0 { None } else { Some(s) };
            do_sync_send(tx, &v);
            Ok(count != 0)
        }

        match (std::mem::take(&mut self.get_buf), std::mem::take(&mut self.int_tx)) {

            (Some(get_buf), Some(int_tx)) => {
                std::mem::swap(&mut self.jh, &mut Some(std::thread::spawn(move || {
                    let mut buf = std::io::BufReader::new(get_buf());
                    loop {
                        match command_read_std(&mut buf, &int_tx) {
                            Ok(false) => {
                                return ();
                            },
                            Ok(true) => { },
                            Err(e) => {
                                panic!("IO Error reading TAP {:?}", e);
                            },
                        }
                    }
                })));
            },

            _ => (),

        }
    }

    fn take_pending(&mut self) -> Option<Line> {
        std::mem::take(&mut self.pending)
    }

    fn put_pending(&mut self, line: Line) {
        if self.pending.is_some() {
            panic!(
                "Tried to put line '{:?}' but line '{:?}' is already in there",
                line,
                self.pending
            );
        }
        std::mem::replace(&mut self.pending, Some(line));
    }

    fn get_transmitter(&mut self) -> &SyncSender<Line> {
        &self.tx
    }

    fn do_pending_send(&mut self, ps: &mut ProcessStatus) {
        let line = match self.take_pending() {
            Some(line) => line,
            None => { return (); }
        };
        let is_none = line.is_none();
        match self.get_transmitter().try_send(line) {
            Err(TrySendError::Disconnected(l)) | Err(TrySendError::Full(l))  => {
                self.put_pending(l);
                ps.set_stopped_by(StoppedBy::OutputFull);
            },
            Ok(_) => {
                ps.set_stopped_by(StoppedBy::StillWorking);
                if !is_none {
                    ps.add_to_wrote_to(vec![0].into_iter());
                } else {
                    ps.set_stopped_by(StoppedBy::ExhaustedInput);
                }
            }
        }
    }

}

impl <R: 'static +  std::io::Read> ProcessableItem for TapProcessor<R> {
    fn process(&mut self) -> ProcessStatus {


        let mut ps = ProcessStatus::new();

        loop {
            self.do_pending_send(&mut ps);
            match &ps.stopped_by {
                StoppedBy::StillWorking => (),
                _ => {
                    return ps;
                }
            }
            match self.int_rx.try_recv() {
                Ok(line) => {
                    self.put_pending(line);
                },
                _ => {
                    ps.set_stopped_by(StoppedBy::Waiting);
                },
            };
        }

    }
}


pub struct Sink<O> where O: std::io::Write {
    input: Option<Receiver<Line>>,
    get_buf: Option<fn() -> O>,
}

impl <O: std::io::Write> Sink<O> where {
    pub fn new(get_buf: fn() -> O) -> Sink<O> {
        Sink {
            input: None,
            get_buf: Some(get_buf),
        }
    }

    pub fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError> {
        if self.input.is_none() {
            std::mem::replace(&mut self.input, Some(input));
            return Ok(())
        }
        Err(InputAlreadyUsedError::InputAlreadyUsedError)
    }
}

impl <W: 'static +  std::io::Write> GetProcessable<SinkProcessor<W>> for Sink<W> {
    fn get_processor(&mut self) -> Result<SinkProcessor<W>, ProcessorAlreadyUsedError> {

        match (std::mem::take(&mut self.input), std::mem::take(&mut self.get_buf)) {
            (Some(rx), Some(get_buf)) => {
                Ok(SinkProcessor::new(get_buf, rx))
            },
            _ => Err(ProcessorAlreadyUsedError::ProcessorAlreadyUsedError)
        }

    }
}


pub struct SinkProcessor<W> where W: std::io::Write {
    rx: Receiver<Line>,
    get_w: Option<fn() -> W>,
    jh: Option<JoinHandle<()>>,
    int_tx: SyncSender<Line>,
    int_rx: Option<Receiver<Line>>,
    pending: Option<Line>,
}

impl <W: 'static +  std::io::Write> SinkProcessor<W> {

    fn new(w: fn() -> W, rx: Receiver<Line>) -> SinkProcessor<W> {
        let (int_tx, int_rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
        let mut r = SinkProcessor { get_w: Some(w), rx, jh: None, int_tx: int_tx, int_rx: Some(int_rx), pending: None };
        r.setup();
        r
    }

    fn setup(&mut self) {

        match (std::mem::take(&mut self.int_rx), std::mem::take(&mut self.get_w)) {
            (Some(int_rx), Some(get_buf)) => {
                std::mem::swap(&mut self.jh, &mut Some(std::thread::spawn(move || {
                    let mut wr = std::io::BufWriter::new(get_buf());
                    loop {
                        match int_rx.recv() {
                            Ok(None) => {
                                wr.flush();
                                return ();
                            }
                            Ok(Some(line)) => {
                                match wr.write_all(line.as_bytes()) {
                                    Err(e) => {
                                        return ();
                                    }
                                    _ => {},
                                }
                            }
                            Err(_) => {
                                wr.flush();
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            },
                        }
                    }
                })));
            },
            _ => (),
        }

    }

    fn do_send(&mut self, mut ps: ProcessStatus) -> ProcessStatus {
        let line = match std::mem::take(&mut self.pending) {
            Some(line) => line,
            None => { return ps; }
        };
        let is_none = line.is_none();
        match self.int_tx.try_send(line) {
            Err(TrySendError::Full(l)) | Err(TrySendError::Disconnected(l)) => {
                std::mem::replace(&mut self.pending, Some(l));
                ps.set_stopped_by(StoppedBy::OutputFull);
            },
            Ok(_) => {
                ps.set_stopped_by(StoppedBy::StillWorking);
                if !is_none {
                    ps.add_to_read_from(vec![0].into_iter());
                } else {
                    ps.set_stopped_by(StoppedBy::ExhaustedInput);
                }
            }
        }
        ps
    }

}


impl <W: 'static +  std::io::Write> ProcessableItem for SinkProcessor<W> {
    fn process(&mut self) -> ProcessStatus {

        if self.get_w.is_some() {
            panic!("What am I doing here");
        }

        let mut ps = ProcessStatus::new();


        if self.pending.is_some() {
            ps = self.do_send(ps);
        }

        if self.pending.is_some() {
            return ps;
        }


        loop {
            match self.rx.try_recv() {
                Ok(line) => {
                    std::mem::replace(&mut self.pending, Some(line));
                    ps = self.do_send(ps);
                    // ps
                }
                _ => {
                    ps.set_stopped_by(StoppedBy::Waiting);
                    // ps
                }
            }

            match ps.stopped_by {
                StoppedBy::StillWorking => (),
                _ => { return ps; }
            }
        }

    }
}


pub enum CommandOutput {
    Stdout,
    Stderr,
}


#[derive(Debug)]
pub struct Command<E, A, O, K, V, P>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = O>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    command: O,
    path: P,
    env: Option<E>,
    args: Option<A>,
    stdout: Option<SyncSender<Line>>,
    stderr: Option<SyncSender<Line>>,
    stdin: Option<Receiver<Line>>,
}

impl <E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = O>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>> Command<E, A, O, K, V, P> {

    pub fn new(command: O, path: P, env: E, args: A) -> Command<E, A, O, K, V, P> {
        Command { command, path, env: Some(env), args: Some(args), stdin: None, stdout: None, stderr: None }
    }

    pub fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError> {
        if self.stdin.is_none() {
            std::mem::replace(&mut self.stdin, Some(input));
            return Ok(());
        }
        Err(InputAlreadyUsedError::InputAlreadyUsedError)
    }

    pub fn get_output(&mut self, s: CommandOutput) -> Result<Receiver<Line>, OutputAlreadyUsedError> {

        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);

        match s {
            CommandOutput::Stderr => {
                match &self.stderr {
                    Some(_) => Err(OutputAlreadyUsedError::OutputAlreadyUsedError),
                    None => {
                        std::mem::replace(&mut self.stderr, Some(tx));
                        Ok(rx)
                    }
                }
            },
            CommandOutput::Stdout => {
                match &self.stdout {
                    Some(_) => Err(OutputAlreadyUsedError::OutputAlreadyUsedError),
                    None => {
                        std::mem::replace(&mut self.stdout, Some(tx));
                        Ok(rx)
                    }
                }
            },
        }

    }

}

impl <E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = O>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>> GetProcessable<CommandProcessor> for Command<E, A, O, K, V, P> {

    fn get_processor(&mut self) -> Result<CommandProcessor, ProcessorAlreadyUsedError> {

        let o_child = match (std::mem::take(&mut self.env), std::mem::take(&mut self.args)) {
            (Some(env), Some(args)) => {
                std::process::Command::new(&self.command)
                    .current_dir(&self.path)
                    .args(args.into_iter())
                    .envs(env.into_iter())
                    .stdin(if self.stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() } )
                    .stderr(if self.stderr.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                    .stdout(if self.stdout.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
                    .spawn().ok()
            },
            _ => None
        };

        match o_child {
            Some(child) => Ok(CommandProcessor::new(
                    child.stdin,
                    child.stdout,
                    child.stderr,
                    std::mem::take(&mut self.stdin),
                    std::mem::take(&mut self.stdout),
                    std::mem::take(&mut self.stderr),
            )),
            _ => Err(ProcessorAlreadyUsedError::ProcessorAlreadyUsedError)
        }

    }
}

#[derive(Debug)]
pub struct CommandProcessor {
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    stdin_rx: Option<Receiver<Line>>,
    stdout_tx: Option<SyncSender<Line>>,
    stderr_tx: Option<SyncSender<Line>>,
    stdin_joinhandle: Option<JoinHandle<()>>,
    stdout_joinhandle: Option<JoinHandle<()>>,
    stderr_joinhandle: Option<JoinHandle<()>>,
    inner_stdin_tx: Option<SyncSender<Line>>,
    inner_stdout_rx: Option<Receiver<Line>>,
    inner_stderr_rx: Option<Receiver<Line>>,
    pending_stdin: Option<Line>,
    pending_stdout: Option<Line>,
    pending_stderr: Option<Line>,
}

impl CommandProcessor {


    fn new(
        stdin: Option<std::process::ChildStdin>,
        stdout: Option<std::process::ChildStdout>,
        stderr: Option<std::process::ChildStderr>,
        stdin_rx: Option<Receiver<Line>>,
        stdout_tx: Option<SyncSender<Line>>,
        stderr_tx: Option<SyncSender<Line>>) -> CommandProcessor
    {
        let mut cmdp = CommandProcessor {
            stdin,
            stdout,
            stderr,
            stdin_rx,
            stdout_tx,
            stderr_tx,
            stdin_joinhandle: None,
            stdout_joinhandle: None,
            stderr_joinhandle: None,
            inner_stdin_tx: None,
            inner_stdout_rx: None,
            inner_stderr_rx: None,
            pending_stdin: None,
            pending_stdout: None,
            pending_stderr: None,
        };
        cmdp.setup();
        cmdp
    }

    fn setup(&mut self) {

        fn command_read_stdxxx<R>(br: &mut R, tx: &SyncSender<Line>) -> std::io::Result<bool>
            where R: std::io::BufRead
        {
            let mut s = String::new();
            let count = br.read_line(&mut s)?;
            let v = if count == 0 { None } else { Some(s) };
            do_sync_send(tx, &v);
            Ok(count != 0)
        }

        let stdin_joinhandle = match std::mem::take(&mut self.stdin) {
            Some(mut stdin) => {
                let (inner_stdin_tx, inner_stdin_rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
                std::mem::swap(&mut self.inner_stdin_tx, &mut Some(inner_stdin_tx));
                Some(std::thread::spawn(move || {
                    loop {
                        match write(&inner_stdin_rx, &mut stdin) {
                            Ok(SinkWriteStatus::FINISHED) => {
                                println!("COMMAND STDIN END");
                                return;
                            },
                            Ok(SinkWriteStatus::ONGOING) => {
                            },
                            Err(SinkWriteError::TryRecvError(_)) => {
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            },
                            Err(SinkWriteError::WriteError(e)) => {
                                println!("COMMAND STDIN ERROR WRITING: {}", e);
                                return;
                            }
                        }
                    }
                }))
            },
            _ => None,
        };
        std::mem::replace(&mut self.stdin_joinhandle, stdin_joinhandle);


        let stdout_joinhandle = match std::mem::take(&mut self.stdout) {
            Some(stdout) => {
                let (inner_stdout_tx, inner_stdout_rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
                std::mem::swap(&mut self.inner_stdout_rx, &mut Some(inner_stdout_rx));
                Some(std::thread::spawn(move || {
                    let mut br = std::io::BufReader::new(stdout);
                    loop {
                        match command_read_stdxxx(&mut br, &inner_stdout_tx) {
                            Ok(false) => {
                                println!("COMMAND STDOUT FALSE");
                                return;
                            }
                            Ok(_) => {},
                            Err(e) => {
                                println!("COMMAND STDOUT ERROR: {}", e);
                                return;
                            }
                        }
                    }
                }))
            },
            _ => None,
        };
        std::mem::replace(&mut self.stdout_joinhandle, stdout_joinhandle);


        let stderr_joinhandle = match std::mem::take(&mut self.stderr) {
            Some(stderr) => {
                let (inner_stderr_tx, inner_stderr_rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
                std::mem::swap(&mut self.inner_stderr_rx, &mut Some(inner_stderr_rx));
                Some(std::thread::spawn(move || {
                    let mut br = std::io::BufReader::new(stderr);
                    loop {
                        match command_read_stdxxx(&mut br, &inner_stderr_tx) {
                            Ok(false) => {
                                println!("COMMAND STDOUT FALSE");
                                return;
                            }
                            Ok(_) => {},
                            Err(e) => {
                                println!("COMMAND STDOUT ERROR: {}", e);
                                return;
                            }
                        }
                    }
                }))
            },
            _ => None,
        };
        std::mem::replace(&mut self.stderr_joinhandle, stderr_joinhandle);

    }



    fn do_pending(&mut self, mut ps: ProcessStatus) -> ProcessStatus {

        enum SendStatus{
            Sent,
            Finished,
            Pending(Line),
        }

        fn trysend(tx: &SyncSender<Line>, v: Line) -> SendStatus {
            let is_none = v.is_none();
            match tx.try_send(v) {
                Ok(_) => {
                    if is_none { SendStatus::Finished } else { SendStatus::Sent }
                }
                Err(TrySendError::Full(v)) => SendStatus::Pending(v),
                Err(TrySendError::Disconnected(v)) => SendStatus::Pending(v),
            }
        }

        // TODO: Fix when fixed - https://github.com/rust-lang/rust/issues/68354
        match std::mem::take(&mut self.pending_stdin) {
            Some(pending_stdin_line) => {
                match &self.inner_stdin_tx {
                    Some(inner_stdin_tx) => {
                        match trysend(inner_stdin_tx, pending_stdin_line) {
                            SendStatus::Pending(v) => {
                                ps.set_stopped_by(StoppedBy::InternallyFull);
                                std::mem::replace(&mut self.pending_stdin, Some(v));
                            },
                            SendStatus::Finished => {
                                std::mem::replace(&mut self.inner_stdin_tx, None);
                            },
                            SendStatus::Sent => {
                                ps.add_to_read_from(vec![0].into_iter());
                            },
                        }
                    }
                    _ => (),
                }
            }
            _ => (),
        };

        // TODO: Fix when fixed - https://github.com/rust-lang/rust/issues/68354
        match std::mem::take(&mut self.pending_stdout) {
            Some(pending_stdout_line) => {
                match &self.stdout_tx {
                    Some(stdout_tx) => {
                        match trysend(stdout_tx, pending_stdout_line) {
                            SendStatus::Pending(v) => {
                                ps.set_stopped_by(StoppedBy::OutputFull);
                                std::mem::replace(&mut self.pending_stdout, Some(v));
                            },
                            SendStatus::Finished => {
                                std::mem::replace(&mut self.stdout_tx, None);
                            },
                            SendStatus::Sent => {
                                ps.add_to_wrote_to(vec![1].into_iter());
                            },
                        }
                    }
                    _ => (),
                }
            }
            _ => (),
        };

        // TODO: Fix when fixed - https://github.com/rust-lang/rust/issues/68354
        match std::mem::take(&mut self.pending_stderr) {
            Some(pending_stderr_line) => {
                match &self.stderr_tx {
                    Some(stderr_tx) => {
                        match trysend(stderr_tx, pending_stderr_line) {
                            SendStatus::Pending(v) => {
                                ps.set_stopped_by(StoppedBy::OutputFull);
                                std::mem::replace(&mut self.pending_stderr, Some(v));
                            },
                            SendStatus::Finished => {
                                std::mem::replace(&mut self.stderr_tx, None);
                            },
                            SendStatus::Sent => {
                                ps.add_to_wrote_to(vec![2].into_iter());
                            },
                        }
                    }
                    _ => (),
                }
            }
            _ => (),
        };

        match (&self.stderr_tx, &self.stdout_tx, &self.inner_stdin_tx) {
            (None, None, None) => {
                ps.set_stopped_by(StoppedBy::ExhaustedInput);
            },
            _ => {},
        }

        ps
    }


}


impl ProcessableItem for CommandProcessor {

    fn process(&mut self) -> ProcessStatus {

        let mut process_status = ProcessStatus::new();

        loop {

            // TODO: Should we do some /all of the accounting here instead... we
            // may want to do it when we put data into the `inner_*_tx` channels

            process_status = self.do_pending(process_status);

            match process_status.stopped_by {
                StoppedBy::StillWorking => (),
                _ => { return process_status; }
            }

            let stdin_processed = match self.stdin_rx.as_ref().and_then(|stdin_rx| { stdin_rx.try_recv().ok() }) {
                Some(line) => {
                    std::mem::swap(&mut self.pending_stdin, &mut Some(line));
                    true
                },
                None => {
                    false
                },
            };

            let stdout_processed = match self.inner_stdout_rx.as_ref().and_then(|i_stdout_rx| { i_stdout_rx.try_recv().ok() }) {
                Some(line) => {
                    std::mem::swap(&mut self.pending_stdout, &mut Some(line));
                    true
                },
                None => {
                    false
                },
            };

            let stderr_processed = match self.inner_stderr_rx.as_ref().and_then(|i_stderr_rx| { i_stderr_rx.try_recv().ok() }) {
                Some(line) => {
                    std::mem::swap(&mut self.pending_stderr, &mut Some(line));
                    true
                },
                None => {
                    false
                },
            };

            match (stdin_processed, stdout_processed, stderr_processed) {
                (false, false, false) => { process_status.set_stopped_by(StoppedBy::Waiting); },
                _ => (),
            }

        }
    }

}


#[derive(Debug)]
pub enum ProcessorAlreadyUsedError {
    ProcessorAlreadyUsedError,
}

impl std::fmt::Display for ProcessorAlreadyUsedError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ProcessorAlreadyUsedError::ProcessorAlreadyUsedError => write!(f, "ProcessorAlreadyUsedError"),
        }
    }
}


pub struct Buffer {
    input: Vec<(usize, Receiver<Line>)>,
    int: Option<(Sender<Line>, Receiver<Line>)>,
    output: Vec<SyncSender<Line>>,
    buffer_size_output: Option<Sender<usize>>,
}


impl Buffer {

    pub fn new() -> Buffer {

        let internal: (Sender<Line>, Receiver<Line>) = channel();

        Buffer {
            input: vec![],
            int: Some(internal),
            output: vec![],
            buffer_size_output: None,
        }

    }

    pub fn add_output(&mut self) -> Result<Receiver<Line>, ProcessorAlreadyUsedError> {

        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
        match &self.int {
            Some(_) => {
                self.output.push(tx);
                Ok(rx)
            },
            None => return Err(ProcessorAlreadyUsedError::ProcessorAlreadyUsedError),
        }

    }

    pub fn add_input(&mut self, priority: usize, rx: Receiver<Line>) -> Result<(), ProcessorAlreadyUsedError> {
        match &self.int {
            Some(_vec) => {
                self.input.push((priority, rx));
                Ok(())
            },
            None => return Err(ProcessorAlreadyUsedError::ProcessorAlreadyUsedError),
        }
    }

}

pub trait GetProcessable<P> {
    fn get_processor(&mut self) -> Result<P, ProcessorAlreadyUsedError>;
}

impl GetProcessable<BufferProcessor> for Buffer {

    fn get_processor(&mut self) -> Result<BufferProcessor, ProcessorAlreadyUsedError> {
        match (std::mem::take(&mut self.int), std::mem::take(&mut self.buffer_size_output)) {
            (Some(_int), _buffer_size_output) => {
                let mut output: Vec<SyncSender<Line>> = vec![];
                output.append(&mut self.output);
                Ok(BufferProcessor::new(
                    &mut self.input,
                    // int,
                    output,
                    // buffer_size_output,
                ))
            },
            _ => Err(ProcessorAlreadyUsedError::ProcessorAlreadyUsedError),
        }
    }


}


#[derive(Debug)]
pub struct PendingMessage (usize, Line);

#[derive(Debug)]
pub struct BufferProcessor {
    positions: Vec<usize>,
    lengths: Vec<usize>,
    input: Vec<Vec<Option<Receiver<Line>>>>,
    output: Vec<SyncSender<Line>>,
    partially_sent: Option<PendingMessage>,
}

impl BufferProcessor {

    pub fn new(mut input: &mut Vec<(usize, Receiver<Line>)>, output: Vec<SyncSender<Line>>) -> BufferProcessor {
        let new_input = BufferProcessor::organize_input(&mut input);
        let positions = BufferProcessor::get_lengths(&new_input).iter().map(|n| n - 1).collect();
        let lengths = BufferProcessor::get_lengths(&new_input);

        BufferProcessor {
            input: new_input,
            positions,
            output,
            lengths,
            partially_sent: None,
        }
    }

    fn organize_input<X>(input: &mut Vec<(usize, X)>) -> Vec<Vec<Option<X>>> {

        input.sort_by(|(a, _), (b, _)| a.cmp(b));

        struct Acc<X> {
            priority: usize,
            current: Vec<X>,
            result: Vec<Vec<X>>,
        }

        let mut acc: Acc<Option<X>> = Acc {
            priority: input[input.len() - 1].0,
            current: vec![],
            result: vec![],
        };

        fn shuffle_acc<Z>(a: &mut Acc<Z>) {
            let mut v: Vec<Z> = vec![];
            v.append(&mut a.current);
            v.reverse();
            a.result.push(v);
        }

        while input.len() > 0 {
            let (n, x) = input.pop().unwrap();
            if n != acc.priority {
                shuffle_acc(&mut acc);
            }
            acc.priority = n;
            acc.current.push(Some(x));
        }

        if acc.current.len() > 0 {
            shuffle_acc(&mut acc);
        }
        acc.result
    }

    fn get_priority_sequence(lengths: &Vec<usize>, positions: &Vec<usize>, priority: usize) -> Vec<usize> {

        let mut r = vec![9; lengths[priority]];
        let d = |i| if i == 0 { lengths[priority] - 1 } else { i - 1 };
        let mut n = d(positions[priority]);

        for i in 0..lengths[priority] {
            n = d(n);
            r[i] = n;
        }

        r
    }

    fn mark_position(positions: &mut Vec<usize>, priority: usize, got_data_from: usize) {
        positions[priority] = got_data_from;
    }


    fn get_lengths<X>(input: &Vec<Vec<X>>) -> Vec<usize> {
        input.iter().map(|inp| inp.len()).collect()
    }


    fn no_inputs_left(input: &Vec<Vec<Option<Receiver<Line>>>>) -> bool {
        for v in input {
            for o in v {
                if o.is_some() { return false }
            }
        }
        true
    }


    fn get_line(rx: &Vec<Vec<Option<Receiver<Line>>>>, lengths: &Vec<usize>, positions: &Vec<usize>) -> Result<(usize, usize, Line), TryRecvError> {

        fn mapper(o: &Option<Receiver<Line>>) -> Result<Line, TryRecvError> {
            match o {
                None => Err(TryRecvError::Empty),
                Some(r) => r.try_recv(),
            }
        }

        for priority in 0..rx.len() {
            let sequence = BufferProcessor::get_priority_sequence(&lengths, &positions, priority);
            for pos in sequence {
                match mapper(&rx[priority][pos]) {
                    Ok(d) => { return Ok((priority, pos, d)); }
                    Err(_) => {},
                }
            }
        }
        Err(TryRecvError::Empty)
    }


    fn send(tx: &Vec<SyncSender<Line>>, pm: &PendingMessage) -> Option<usize> {
        for i in pm.0..tx.len() {
            match tx[i].try_send(pm.1.clone()) {
                Ok(_) => {},
                Err(_) => {
                    return Some(i)
                }
            }
        }
        None
    }

    /**
     * Will send any message which is half sent returning the indices of the
     * outputs where sending has occurred.
     */
    fn send_if_required(&mut self) -> Range<usize> {

        if self.partially_sent.is_none() {
            return 0..0;
        }

        let skip_accounting = match &self.partially_sent {
            Some(PendingMessage(_, None)) => true,
            _ => false,
        };

        let begin = match &self.partially_sent {
            Some(PendingMessage(n, _)) => *n,
            _ => 0,
        };

        let progress = match &self.partially_sent {
            Some(pm) => {
                BufferProcessor::send(&self.output, pm)
            }
            None => None,
        };

        let new_pending = match (progress, std::mem::take(&mut self.partially_sent)) {
           (Some(n), Some(PendingMessage(_, msg))) => Some(PendingMessage(n, msg)),
           _ => None,
        };

        let end = match &new_pending {
            Some(PendingMessage(n, _)) => *n,
            None => self.output.len(),
        };

        std::mem::replace(
            &mut self.partially_sent,
            new_pending
        );

        if skip_accounting {
            return 0..0
        }

        begin..end

    }


    fn priority_position_to_input_index(&self, priority: usize, position: usize) -> usize {
        let mut r: usize = 0;
        for _i in 0..priority {
            r = r + self.input.len();
        }
        r + position
    }


    fn mark_input_empty(&mut self, priority: usize, pos: usize) {
        std::mem::replace(&mut self.input[priority][pos], None);
    }

}

pub trait ProcessableItem {
    fn process(&mut self) -> ProcessStatus;
}


impl ProcessableItem for BufferProcessor {

    fn process(&mut self) -> ProcessStatus {

        let mut process_status = ProcessStatus::new();

        loop {
            process_status.add_to_wrote_to(self.send_if_required());

            if self.partially_sent.is_some() {
                process_status.set_stopped_by(StoppedBy::OutputFull);
                return process_status;
            }

            if BufferProcessor::no_inputs_left(&self.input) {
                process_status.set_stopped_by(StoppedBy::ExhaustedInput);
                return process_status;
            }

            match BufferProcessor::get_line(&self.input, &self.lengths, &self.positions) {
                Err(TryRecvError::Empty) => {
                    process_status.set_stopped_by(StoppedBy::Waiting);
                    return process_status;
                }
                Err(TryRecvError::Disconnected) => { panic!("This should not be possible"); },
                Ok((priority, pos, None)) => {
                    // self.input[priority].remove(pos);
                    // self.lengths[priority] = self.lengths[priority] - 1;
                    self.mark_input_empty(priority, pos);
                    if BufferProcessor::no_inputs_left(&self.input) {
                        std::mem::replace(&mut self.partially_sent, Some(PendingMessage(0, None)));
                    }
                },
                Ok((priority, pos, Some(s))) => {
                    BufferProcessor::mark_position(&mut self.positions, priority, pos);
                    std::mem::replace(&mut self.partially_sent, Some(PendingMessage(0, Some(s))));
                    process_status.add_to_read_from(vec![self.priority_position_to_input_index(priority, pos)].into_iter());
                },
            }
        }

    }

}


#[test]
fn can_buffer_organize_input() {
    let mut input = vec![(5,99),(5,12),(3, 33),(1,1)];
    assert_eq!(
        vec![vec![Some(99),Some(12)],vec![Some(33)],vec![Some(1)]],
        BufferProcessor::organize_input(&mut input)
    );
}


#[test]
fn can_buffer_processor_dec_initial() {
    let lengths = vec![2, 5, 2];
    let positions = vec![1, 4, 1];
    assert_eq!(
        vec![1, 0],
        BufferProcessor::get_priority_sequence(&lengths, &positions, 0)
    );
}


#[test]
fn get_line_test() {
    let lengths = vec![2, 3, 2];
    let positions = vec![1, 2, 1];

    fn get_channel(c: usize) -> (Vec<SyncSender<Line>>, Vec<Receiver<Line>>) {
        let mut t = vec![];
        let mut r = vec![];
        for _i in 0..c {
            let (tx, rx) = sync_channel(1);
            t.push(tx);
            r.push(rx);
        }
        (t, r)
    }

    let mut channels = get_channel(7);

    let inputs: Vec<Vec<Option<Receiver<Line>>>> = vec![
        vec![Some(channels.1.pop().unwrap()), Some(channels.1.pop().unwrap())],
        vec![Some(channels.1.pop().unwrap()), Some(channels.1.pop().unwrap()), Some(channels.1.pop().unwrap())],
        vec![Some(channels.1.pop().unwrap()), Some(channels.1.pop().unwrap())],
    ];

    assert_eq!(
        true,
        channels.0[3].send(Some("HI".to_owned())).is_ok()
    );

    assert_eq!(
        Ok((1, 1, Some("HI".to_owned()))),
        BufferProcessor::get_line(&inputs, &lengths, &positions)
    );

}

#[test]
fn test_send_if_required() {

    let input = vec![];
    let positions = vec![];
    let lengths = vec![];
    let mut out_tx = vec![];
    let mut out_rx = vec![];

    for _i in 0..6 {
        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
        out_tx.push(tx);
        out_rx.push(rx);
    }

    out_tx[4].send(Some("Bye".to_owned())).unwrap();

    let mut bp = BufferProcessor {
        input,
        positions,
        output: out_tx,
        lengths,
        partially_sent: Some(PendingMessage(1, Some("HI".to_owned()))),
    };

    assert_eq!(1..4, bp.send_if_required());
    assert_eq!(Err(TryRecvError::Empty), out_rx[0].try_recv());
    assert_eq!(Ok(Some("HI".to_owned())), out_rx[1].try_recv());
    assert_eq!(Ok(Some("HI".to_owned())), out_rx[2].try_recv());
    assert_eq!(Ok(Some("HI".to_owned())), out_rx[3].try_recv());
    assert_eq!(Ok(Some("Bye".to_owned())), out_rx[4].try_recv());
    assert_eq!(Err(TryRecvError::Empty), out_rx[5].try_recv());

    assert_eq!(4..6, bp.send_if_required());
    assert_eq!(Ok(Some("HI".to_owned())), out_rx[5].try_recv());

    assert_eq!(0..0, bp.send_if_required());

}



