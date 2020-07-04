use std::path::Path;
use std::process::Stdio;
use std::ffi::OsStr;
use std::process::Command;
use std::io::Write;
use std::sync::mpsc::{sync_channel, SyncSender, SendError, RecvError, TryRecvError, Receiver};

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

type SendLineError = SendError<std::option::Option<std::string::String>>;

fn do_send(tx: &SyncSender<Option<String>>, msg: &Option<String>) {
    loop {
        match tx.send(msg.clone()) {
            Ok(_) => return (),
            Err(_) => (),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}


pub fn fan_write(input: &Receiver<Option<String>>, senders: &mut Vec<SyncSender<Option<String>>>) -> usize {

    match input.try_recv() {
        Err(TryRecvError::Empty) => {
            return senders.len();
        }
        Err(TryRecvError::Disconnected) => {
            return senders.len();
        }
        Ok(None) => {
            for i in (0..senders.len()).rev() {
                do_send(&senders[i], &None);
                senders.remove(i);
            }
        }
        Ok(line) => {
            for i in (0..senders.len()).rev() {
                do_send(&senders[i], &line)
            }
        }
    };

    senders.len()
}


pub fn get_channels(count: usize) -> (Vec<SyncSender<Option<String>>>, Vec<Receiver<Option<String>>>) {

    let mut res_send: Vec<SyncSender<Option<String>>> = vec![];
    let mut res_recv: Vec<Receiver<Option<String>>> = vec![];

    for _i in 0..count {
        let (tx, rx): (SyncSender<Option<String>>, Receiver<Option<String>>) = sync_channel(1);
        res_send.push(tx);
        res_recv.push(rx);
    }

    (res_send, res_recv)

}



pub fn fan(input: Receiver<Option<String>>, count: usize) -> Vec<Receiver<Option<String>>> {

    let (mut res_send, res_recv) = get_channels(count);

    std::thread::spawn(move || {
        loop {
            fan_write(&input, &mut res_send);
            std::thread::sleep(std::time::Duration::from_millis(10))
        }
    });

    res_recv
}

pub fn funnel_write(inputs: &mut Vec<Receiver<Option<String>>>, sender: &SyncSender<Option<String>>) -> usize {

    for i in (0..inputs.len()).rev() {
        match &inputs[i].try_recv() {
            Ok(None) => {
                &inputs.remove(i);
            }
            Ok(d) => {
                do_send(&sender, &d);
            },
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Disconnected) => (),
        }
    }

    if inputs.len() == 0 {
        do_send(&sender, &None);
    }

    inputs.len()
}




pub fn funnel(mut input: Vec<Receiver<Option<String>>>) -> Receiver<Option<String>> {

    let (tx, rx): (SyncSender<Option<String>>, Receiver<Option<String>>) = sync_channel(1);

    std::thread::spawn(move || {
        loop {
            // TODO: Check that we have any inputs left to write to!
            funnel_write(&mut input, &tx);
            if input.len() == 0 {
                // TODO: Means all inputs have disconnected
                return ();
            }
            std::thread::sleep(std::time::Duration::from_millis(10))
        }
    });

    rx

}


fn command_read_std<R>(br: &mut R, tx: &SyncSender<Option<String>>) -> std::io::Result<bool>
    where R: std::io::BufRead
{
    let mut s = String::new();
    let count = br.read_line(&mut s)?;
    let v = if count == 0 { None } else { Some(s) };
    do_send(tx, &v);
    Ok(count != 0)
}


pub fn tap<R: 'static>(get_buf_reader: fn() -> R) -> Receiver<Option<String>>
    where R: std::io::Read
{

    let (tx, rx): (SyncSender<Option<String>>, Receiver<Option<String>>) = sync_channel(1);

    std::thread::spawn(move || {
        let mut br = std::io::BufReader::new(get_buf_reader());
        loop {
            match command_read_std(&mut br, &tx) {
                Ok(false) => return (),
                Ok(true) => (),
                Err(e) => {
                    println!("TAP: READ: ERROR: {:?}", e);
                    return ();
                }
            };
            std::thread::sleep(std::time::Duration::from_millis(10))
        }
    });

    rx
}

#[derive(Debug)]
pub enum SinkWriterError {
    TryRecvError(TryRecvError),
    WriteError(std::io::Error),
}

pub fn sink_write<W>(i: &Receiver<Option<String>>, w: &mut W) -> Result<bool, SinkWriterError>
    where W: std::io::Write
{

    match i.try_recv() {
        Ok(None) => {
            match w.flush() {
                Ok(()) => Ok(false),
                Err(e) => Err(SinkWriterError::WriteError(e)),
            }
        },
        Ok(Some(line)) => {
            w.write(line.as_bytes())
                .map(|_x| true)
                .map_err(|e| SinkWriterError::WriteError(e))
        },
        Err(x) => {
            Err(SinkWriterError::TryRecvError(x))
        }
    }

}

pub fn sink<O: 'static>(i: Receiver<Option<String>>, get_buf_write: fn() -> O)
    where O: std::io::Write 
{

    std::thread::spawn(move || {
        let mut wr = std::io::BufWriter::new(get_buf_write());
        loop {
            match sink_write(&i, &mut wr) {
                Ok(false) => {
                    return ();
                },
                Ok(true) => (),
                Err(SinkWriterError::TryRecvError(TryRecvError::Disconnected)) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                },
                Err(SinkWriterError::WriteError(e)) => {
                    println!("SINK: WRITE ERR: {:?}", &e);
                    return ();
                }
                Err(SinkWriterError::TryRecvError(TryRecvError::Empty)) => {
                    // TODO: Should we flush here?
                    // wr.flush();
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    });

}

enum CommandStdWriteResultError {
    RecvError(RecvError),
    IoError(std::io::Error),
}

impl std::fmt::Display for CommandStdWriteResultError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CommandStdWriteResultError::IoError(e) => write!(f, "IoError: {}", e),
            CommandStdWriteResultError::RecvError(e) => write!(f, "RecvError: {}", e),
        }
    }
}


fn command_write_std<W>(input: &Receiver<Option<String>>, bw: &mut W) -> Result<bool, CommandStdWriteResultError>
    where W: std::io::Write
{
    let eol: &str = "\n";
    let line = input.recv().map_err(|e| CommandStdWriteResultError::RecvError(e))?;
    match line {
        None => Ok(false),
        Some(mut s) => {
            s.push_str(eol);
            bw.write_all(s.as_bytes()).map_err(|e| CommandStdWriteResultError::IoError(e))?;
            Ok(true)
        }
    }
}


pub fn command<E, A, O, K, V, P>(input: Receiver<Option<String>>, command: O, path: P, env: E, args: A) -> std::io::Result<(Receiver<Option<String>>, Receiver<Option<String>>)>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = O>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{

    let (out_tx, out_rx): (SyncSender<Option<String>>, Receiver<Option<String>>) = sync_channel(1);
    let (err_tx, err_rx): (SyncSender<Option<String>>, Receiver<Option<String>>) = sync_channel(1);

    let child = Command::new(command)
        .current_dir(path)
        .args(args)
        .envs(env)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    match child.stdin {
        None => (),
        Some(mut stdin) => {
            std::thread::spawn(move || {
                loop {
                    match command_write_std(&input, &mut stdin) {
                        Ok(false) => {
                            return;
                        }
                        Ok(_) => (),
                        Err(e) => {
                            println!("{}", e);
                            return;
                        }
                    }
                }
            });
        }
    };

    match child.stderr {
        None => (),
        Some(stderr) => {
            std::thread::spawn(move || {
                let mut br = std::io::BufReader::new(stderr);
                loop {
                    match command_read_std(&mut br, &err_tx) {
                        Ok(false) => {
                            return;
                        }
                        Ok(_) => (),
                        Err(e) => {
                            println!("{}", e);
                            return;
                        }
                    }
                }
            });
        }
    }

    match child.stdout {
        None => (),
        Some(stdout) => {
            std::thread::spawn(move || {
                let mut br = std::io::BufReader::new(stdout);
                loop {
                    match command_read_std(&mut br, &out_tx) {
                        Ok(false) => {
                            return;
                        }
                        Ok(_) => (),
                        Err(e) => {
                            println!("{}", e);
                            return;
                        }
                    }
                }
            });
        }
    }

    return Ok((out_rx, err_rx));
}


#[test]
fn can_tap() {
    // let get_input = || FakeStdin::new("hi".to_owned());
    let mut input = std::io::BufReader::new(FakeStdin::new("hi jack".to_owned()));
    let (inp_tx, inp_rx): (SyncSender<Option<String>>, Receiver<Option<String>>) = sync_channel(1);

    let (mut fan_tx, mut fan_rx) = get_channels(3);
    let (fun_tx, fun_rx): (SyncSender<Option<String>>, Receiver<Option<String>>) = sync_channel(3);

    assert_eq!(command_read_std(&mut input, &inp_tx).unwrap_or(false), true);
    fan_write(&inp_rx, &mut fan_tx);

    let mut stdout = FakeStdout::new();
    assert_eq!(funnel_write(&mut fan_rx, &fun_tx), 3);

    assert_eq!(sink_write(&fun_rx, &mut stdout).unwrap_or(false), true);
    assert_eq!(sink_write(&fun_rx, &mut stdout).unwrap_or(false), true);
    assert_eq!(sink_write(&fun_rx, &mut stdout).unwrap_or(false), true);


    assert_eq!(stdout.received.len(), 21);

}


