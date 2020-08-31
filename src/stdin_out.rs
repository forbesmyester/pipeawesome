
#[derive(Debug)]
pub enum TapMethod {
    STDIN,
    FILENAME(String),
}

#[derive(Debug, Clone)]
pub enum SinkMethod {
    STDOUT,
    STDERR,
    FILENAME(String)
}

pub trait GetRead {
    type R: std::io::Read;
    fn get_read(self) -> Self::R;
}

pub trait GetWrite {
    type W: std::io::Write;
    fn get_write(&self) -> Self::W;
}


pub struct StdinGetRead {}
impl GetRead for StdinGetRead {
    type R = std::io::Stdin;
    fn get_read(self) -> std::io::Stdin {
        std::io::stdin()
    }
}

pub struct FileGetRead<P> where P: AsRef<std::path::Path> { p: P, }
impl <P> FileGetRead<P> where P: AsRef<std::path::Path> {
    pub fn new(p: P) -> FileGetRead<P> {
        FileGetRead { p }
    }
}
impl <P> GetRead for FileGetRead<P> where P: AsRef<std::path::Path> {
    type R = std::fs::File;
    fn get_read(self) -> std::fs::File {
        std::fs::File::open(&self.p).unwrap()
    }
}

pub struct StdoutGetWrite {}
impl GetWrite for StdoutGetWrite {
    type W = std::io::Stdout;
    fn get_write(&self) -> std::io::Stdout {
        std::io::stdout()
    }
}

pub struct StderrGetWrite {}
impl GetWrite for StderrGetWrite {
    type W = std::io::Stderr;
    fn get_write(&self) -> std::io::Stderr {
        std::io::stderr()
    }
}

pub struct FileGetWrite<P> where P: AsRef<std::path::Path> { p: P, }
impl <P> FileGetWrite<P> where P: AsRef<std::path::Path> {
    pub fn new(p: P) -> FileGetWrite<P> {
        FileGetWrite { p }
    }
}
impl <P> GetWrite for FileGetWrite<P> where P: AsRef<std::path::Path> {
    type W = std::fs::File;
    fn get_write(&self) -> std::fs::File {
        std::fs::File::create(&self.p).unwrap()
    }
}

