use std::sync::mpsc::channel;
use std::path::Path;
use std::process::Stdio;
use std::ffi::OsStr;
use std::io::Write;
use std::sync::mpsc::{sync_channel, Sender, SyncSender, SendError, RecvError, TrySendError, TryRecvError, Receiver};

type Line = Option<String>;

#[cfg(test)]
struct FakeStdin {
    src: String,
    pos: usize,
    max_read: Option<usize>,
}


#[cfg(test)]
impl FakeStdin {
    pub fn new(strng: String) -> FakeStdin {
        FakeStdin {
            src: strng,
            pos: 0,
            max_read: Option::None,
        }
    }
}


#[cfg(test)]
impl std::io::Read for FakeStdin {

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {

        let mut to_read = self.src.len() - self.pos;

        if to_read > buf.len() {
            to_read = buf.len();
        }

        if to_read > self.max_read.unwrap_or(to_read) {
            to_read = self.max_read.unwrap_or(to_read);
        }

        if to_read == 0 {
            return Result::Ok(0);
        }

        for i in 0..to_read {
            buf[i] = self.src.as_bytes()[i + self.pos];
        }

        self.pos = self.pos + to_read;

        Result::Ok(to_read)

    }

}


pub trait Pullable {
    fn pull(&mut self) -> Result<bool, TryRecvError>;
}

pub trait SingleInput {
    fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError>;
}

pub trait SingleOutput {
    fn get_output(&mut self) -> Result<Receiver<Line>, OutputAlreadyUsedError>;
}

pub trait Processable {
    fn process(&mut self) -> Option<Vec<std::thread::JoinHandle<()>>>;
}


#[cfg(test)]
struct FakeStdout {
    received: Vec<u8>,
}


#[cfg(test)]
impl FakeStdout {
    pub fn new() -> FakeStdout {
        FakeStdout { received: vec![] }
    }
}


#[cfg(test)]
impl std::io::Write for FakeStdout {
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for u in buf {
            self.received.push(*u);
        }
        Ok(buf.len())
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


fn do_send(tx: &Sender<Line>, msg: &Option<String>) {
    loop {
        match tx.send(msg.clone()) {
            Ok(_) => return (),
            Err(_) => (),
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
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


pub fn get_channels(count: usize) -> (Vec<SyncSender<Line>>, Vec<Receiver<Line>>) {

    let mut res_send: Vec<SyncSender<Line>> = vec![];
    let mut res_recv: Vec<Receiver<Line>> = vec![];

    for _i in 0..count {
        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
        res_send.push(tx);
        res_recv.push(rx);
    }

    (res_send, res_recv)

}


// pub fn fan(input: Receiver<Line>, count: usize) -> Vec<Receiver<Line>> {

//     let (mut res_send, res_recv) = get_channels(count);

//     std::thread::spawn(move || {
//         loop {
//             fan_write(&input, &mut res_send);
//             std::thread::sleep(std::time::Duration::from_millis(10))
//         }
//     });

//     res_recv
// }


pub struct Fan {
    input: Option<Receiver<Line>>,
    tx: Vec<SyncSender<Line>>,
}

impl Fan {

    pub fn new() -> Fan {
        Fan { input: None, tx: vec![] }
    }

    pub fn add_output(&mut self) -> Result<Receiver<Line>, OutputAlreadyUsedError> {
        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
        self.tx.push(tx);
        Ok(rx)
    }

}

impl SingleInput for Fan {

    fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError> {
        if self.input.is_none() {
            std::mem::replace(&mut self.input, Some(input));
            return Ok(())
        }
        Err(InputAlreadyUsedError::InputAlreadyUsedError)
    }

}

impl Pullable for Fan {
    fn pull(&mut self) -> Result<bool, TryRecvError> {

        fn fan_write(input: &Receiver<Line>, senders: &Vec<SyncSender<Line>>) -> Result<bool, TryRecvError> {
            let line = input.try_recv()?;
            for i in (0..senders.len()).rev() {
                do_sync_send(&senders[i], &line);
            }
            Ok(senders.len() > 0)
        }

        match (&self.input, &self.tx) {
            (Some(input), tx) => {
                fan_write(&input, &tx)?;
                Ok(false)
            },
            _ => Ok(false)
        }
    }
}


pub struct Funnel {
    tx: Option<SyncSender<Line>>,
    rx: Vec<Receiver<Line>>,
}

impl Funnel {

    pub fn new() -> Funnel {
        Funnel { tx: None, rx: vec![] }
    }

    pub fn add_input(&mut self, rx: Receiver<Line>) {
        self.rx.push(rx);
    }

}

impl Pullable for Funnel {
    fn pull(&mut self) -> Result<bool, TryRecvError> {
        match &self.tx {
            Some(tx) => {

                for i in (0..self.rx.len()).rev() {
                    match self.rx[i].try_recv() {
                        Ok(None) => {
                            self.rx.remove(i);
                            if self.rx.len() == 0 {
                                do_sync_send(&tx, &None);
                            }
                        }
                        Ok(d) => {
                            do_sync_send(&tx, &d);
                            return Ok(true);
                        },
                        Err(TryRecvError::Empty) => { },
                        Err(TryRecvError::Disconnected) => (),
                    }
                }

                Err(TryRecvError::Empty)

            }
            _ => Ok(false)
        }
    }
}

impl SingleOutput for Funnel {
    fn get_output(&mut self) -> Result<Receiver<Line>, OutputAlreadyUsedError> {

        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);

        if self.tx.is_none() {
            std::mem::replace(&mut self.tx, Some(tx));
            return Ok(rx)
        }
        Err(OutputAlreadyUsedError::OutputAlreadyUsedError)
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
}

impl <R: std::io::Read> SingleOutput for Tap<R> {

    fn get_output(&mut self) -> Result<Receiver<Line>, OutputAlreadyUsedError> {

        let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);

        if self.tx.is_none() {
            std::mem::replace(&mut self.tx, Some(tx));
            return Ok(rx)
        }
        Err(OutputAlreadyUsedError::OutputAlreadyUsedError)
    }
}


impl <R: 'static +  std::io::Read> Processable for Tap<R> {
    fn process(&mut self) -> std::option::Option<std::vec::Vec<std::thread::JoinHandle<()>>> {

        fn command_read_std<R>(br: &mut R, tx: &SyncSender<Line>) -> std::io::Result<bool>
            where R: std::io::BufRead
        {
            let mut s = String::new();
            let count = br.read_line(&mut s)?;
            let v = if count == 0 { None } else { Some(s) };
            do_sync_send(tx, &v);
            Ok(count != 0)
        }

        match (std::mem::take(&mut self.tx), std::mem::take(&mut self.get_buf)) {
            (Some(tx), Some(get_buf)) => {
                let t = std::thread::spawn(move || {
                    let mut br = std::io::BufReader::new(get_buf());
                    loop {
                        match command_read_std(&mut br, &tx) {
                            Ok(false) => return (),
                            Ok(true) => (),
                            Err(e) => {
                                println!("TAP: READ: ERROR: {:?}", e);
                                return ();
                            }
                        };
                    }
                });
                Some(vec![t])
            }
            _ => None,
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
}

impl <O: std::io::Write> SingleInput for Sink<O> {
    fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError> {
        if self.input.is_none() {
            std::mem::replace(&mut self.input, Some(input));
            return Ok(())
        }
        Err(InputAlreadyUsedError::InputAlreadyUsedError)
    }
}


impl <O: 'static +  std::io::Write> Processable for Sink<O> {
    fn process(&mut self) -> Option<Vec<std::thread::JoinHandle<()>>> {
        match (std::mem::take(&mut self.input), std::mem::take(&mut self.get_buf)) {
            (Some(input), Some(get_buf)) => {
                let t = std::thread::spawn(move || {
                    let mut wr = std::io::BufWriter::new(get_buf());
                    loop {
                        match write(&input, &mut wr) {
                            Ok(SinkWriteStatus::FINISHED) => {
                                println!("SINK: RETURN");
                                return ();
                            },
                            Ok(SinkWriteStatus::ONGOING) => {
                                // println!("SINK: GO");
                            },
                            Err(SinkWriteError::TryRecvError(TryRecvError::Disconnected)) => {
                                // println!("SINK: DISCO");
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            },
                            Err(SinkWriteError::WriteError(e)) => {
                                println!("SINK: WRITE ERR: {:?}", &e);
                                return ();
                            }
                            Err(SinkWriteError::TryRecvError(TryRecvError::Empty)) => {
                                // TODO: Should we flush here?
                                // println!("SINK: EMPTY");
                                wr.flush();
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                        }
                    }
                });
                Some(vec![t])
            },
            _ => None
        }
    }
}


pub struct Pipe {
    input: Option<Receiver<Line>>,
    tx: Option<SyncSender<Line>>,
}

impl Pipe {
    pub fn new() -> Pipe {
        Pipe { input: None, tx: None }
    }
}

impl SingleOutput for Pipe {

    fn get_output(&mut self) -> Result<Receiver<Line>, OutputAlreadyUsedError> {
        match self.tx {
            None => {
                let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
                std::mem::replace(&mut self.tx, Some(tx));
                Ok(rx)
            },
            Some(_) => Err(OutputAlreadyUsedError::OutputAlreadyUsedError),
        }
    }

}

impl SingleInput for Pipe {

    fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError> {
        if self.input.is_none() {
            std::mem::replace(&mut self.input, Some(input));
            return Ok(())
        }
        Err(InputAlreadyUsedError::InputAlreadyUsedError)
    }

}

impl Pullable for Pipe {
    fn pull(&mut self) -> Result<bool, TryRecvError> {
        match (&self.tx, &self.input) {
            (Some(tx), Some(input)) => {
                let line = input.try_recv()?;
                do_sync_send(&tx, &line);
                Ok(true)
            },
            _ => Ok(false)
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

impl<E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = O>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>> SingleInput for Command<E, A, O, K, V, P> {

    fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError> {
        if self.stdin.is_none() {
            std::mem::replace(&mut self.stdin, Some(input));
            return Ok(());
        }
        Err(InputAlreadyUsedError::InputAlreadyUsedError)
    }

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

    fn inner_process(&mut self, args: A, env: E) -> std::io::Result<Vec<std::thread::JoinHandle<()>>> {

        fn command_read_std_x<R>(br: &mut R, tx: &SyncSender<Line>) -> std::io::Result<bool>
            where R: std::io::BufRead
        {
            let mut s = String::new();
            let count = br.read_line(&mut s)?;
            let v = if count == 0 { None } else { Some(s) };
            do_sync_send(tx, &v);
            Ok(count != 0)
        }

        let mut child = std::process::Command::new(&self.command)
            .current_dir(&self.path)
            .args(args.into_iter())
            .envs(env.into_iter())
            .stdin(if self.stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() } )
            .stderr(if self.stderr.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
            .stdout(if self.stdout.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
            .spawn()?;

        let mut r: Vec<std::thread::JoinHandle<()>> = vec![];

        match (std::mem::take(&mut self.stdin), child.stdin) {
            (Some(mut stdin_rx), Some(mut stdin)) => {
                r.push(std::thread::spawn(move || {
                    loop {
                        match write(&mut stdin_rx, &mut stdin) {
                            Ok(SinkWriteStatus::FINISHED) => {
                                println!("COMMAND STDIN END");
                                return;
                            },
                            Ok(SinkWriteStatus::ONGOING) => { },
                            Err(SinkWriteError::TryRecvError(_)) => {
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            },
                            Err(SinkWriteError::WriteError(e)) => {
                                println!("COMMAND STDIN ERROR WRITING: {}", e);
                                return;
                            }
                        }
                    }
                }));
            }
            _ => (),
        };

        match (std::mem::take(&mut self.stderr), std::mem::take(&mut child.stderr)) {
            (Some(stderr_tx), Some(stderr_strm)) => {
                r.push(std::thread::spawn(move || {
                    let mut br = std::io::BufReader::new(stderr_strm);
                    loop {
                        match command_read_std_x(&mut br, &stderr_tx) {
                            Ok(false) => {
                                println!("COMMAND STDERR FALSE");
                                return;
                            }
                            Ok(_) => {},
                            Err(e) => {
                                println!("COMMAND STDERR ERROR: {}", e);
                                return;
                            }
                        }
                    }
                }));
            },
            _ => (),
        }

        match (std::mem::take(&mut self.stdout), child.stdout) {
            (Some(stdout_tx), Some(stdout_strm)) => {
                r.push(std::thread::spawn(move || {
                    let mut br = std::io::BufReader::new(stdout_strm);
                    loop {
                        match command_read_std_x(&mut br, &stdout_tx) {
                            Ok(false) => {
                                println!("COMMAND STDERR FALSE");
                                return;
                            }
                            Ok(_) => {},
                            Err(e) => {
                                println!("COMMAND STDERR ERROR: {}", e);
                                return;
                            }
                        }
                    }
                }));
            },
            _ => (),
        };

        return Ok(r)

    }

}

impl<E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = O>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>> Processable for Command<E, A, O, K, V, P> {

    fn process(&mut self) -> Option<Vec<std::thread::JoinHandle<()>>> {

        match (&self.env, &self.args) {
            (Some(_), Some(_)) => {
                let a = std::mem::take(&mut self.args);
                let e = std::mem::take(&mut self.env);
                self.inner_process(
                    a.unwrap(), 
                    e.unwrap()
                ).ok()
            },
            _ => None
        }

    }
}


pub struct Buffer {
    input: Option<Receiver<Line>>,
    int: Option<(Sender<Line>, Receiver<Line>)>,
    tx: Option<SyncSender<Line>>,
    bs_tx: Option<Sender<usize>>,
}


impl Buffer {

    pub fn new() -> Buffer {

        let internal: (Sender<Line>, Receiver<Line>) = channel();

        Buffer {
            input: None,
            int: Some(internal),
            tx: None,
            bs_tx: None,
        }

    }

    pub fn get_buffer_size(&mut self) -> Result<Receiver<usize>, OutputAlreadyUsedError> {

        match self.bs_tx {
            Some(_) => Err(OutputAlreadyUsedError::OutputAlreadyUsedError),
            None => {
                let (tx, rx): (Sender<usize>, Receiver<usize>) = channel();
                std::mem::replace(&mut self.bs_tx, Some(tx));
                Ok(rx)
            }
        }

    }

}

impl SingleOutput for Buffer {
    fn get_output(&mut self) -> Result<Receiver<Line>, OutputAlreadyUsedError> {

        match self.tx {
            Some(_) => Err(OutputAlreadyUsedError::OutputAlreadyUsedError),
            None => {
                let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);
                std::mem::replace(&mut self.tx, Some(tx));
                Ok(rx)
            }
        }

    }
}


impl SingleInput for Buffer {
    fn set_input(&mut self, input: Receiver<Line>) -> Result<(), InputAlreadyUsedError> {
        if self.input.is_none() {
            std::mem::replace(&mut self.input, Some(input));
            return Ok(());
        }
        Err(InputAlreadyUsedError::InputAlreadyUsedError)
    }
}

impl Processable for Buffer {

    fn process(&mut self) -> Option<Vec<std::thread::JoinHandle<()>>> {

        let (int_tx, int_rx) = std::mem::take(&mut self.int).unwrap();
        let tx = std::mem::take(&mut self.tx);
        let input = std::mem::take(&mut self.input);
        let o_bs_tx: Option<Sender<usize>> = std::mem::take(&mut self.bs_tx);

        match (tx, input, o_bs_tx) {
            (Some(tx), Some(input), bs_tx) => {

                Some(vec![std::thread::spawn(move || {

                    let mut current_size = 0;

                    let fill = || -> usize {
                        let mut added = 0;
                        loop {

                            match input.try_recv() {
                                Ok(line) => {
                                    do_send(&int_tx, &line);
                                    if line.is_some() {
                                        added = added + 1;
                                    }
                                },
                                Err(TryRecvError::Empty) => {
                                    return added;
                                },
                                Err(TryRecvError::Disconnected) => {
                                    return added;
                                }
                            }
                        }
                    };

                    loop {
                        current_size = current_size + fill();
                        bs_tx.as_ref().and_then(|bt| {
                            bt.send(current_size).ok()
                        });

                        match int_rx.try_recv() {
                            Err(TryRecvError::Empty) => {
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            },
                            Err(TryRecvError::Disconnected) => {
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            },
                            Ok(line) => {
                                if line.is_some() {
                                    current_size = current_size - 1;
                                }
                                bs_tx.as_ref().and_then(|bt| {
                                    bt.send(current_size).ok()
                                });
                                do_sync_send(&tx, &line);
                            },
                        };

                    }
                })])

            },
            _ => None
        }

    }


}


#[test]
fn can_tap() {
    // let get_input = || FakeStdin::new("hi".to_owned());
    let mut input = std::io::BufReader::new(FakeStdin::new("hi jack".to_owned()));
    let (inp_tx, inp_rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(1);

    let (mut fan_tx, mut fan_rx) = get_channels(3);
    let (fun_tx, fun_rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(3);

    assert_eq!(command_read_std(&mut input, &inp_tx).unwrap_or(false), true);
    fan_write(&inp_rx, &mut fan_tx);

    let mut stdout = FakeStdout::new();
    assert_eq!(funnel_write(&mut fan_rx, &fun_tx), 3);

    assert_eq!(write(&fun_rx, &mut stdout).unwrap_or(SinkWriteStatus::ONGOING), SinkWriteStatus::ONGOING);
    assert_eq!(write(&fun_rx, &mut stdout).unwrap_or(SinkWriteStatus::ONGOING), SinkWriteStatus::ONGOING);
    assert_eq!(write(&fun_rx, &mut stdout).unwrap_or(SinkWriteStatus::ONGOING), SinkWriteStatus::ONGOING);


    assert_eq!(stdout.received.len(), 21);

}


