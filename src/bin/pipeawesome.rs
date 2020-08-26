#[path = "../controls.rs"]
mod controls;

#[path = "../config.rs"]
mod config;

#[path = "../accounting.rs"]
mod accounting;

#[path = "../failed.rs"]
mod failed;

use std::thread::JoinHandle;
use std::collections::HashSet;
use petgraph::{ Direction };
use petgraph::dot::Dot;
use clap::{Arg as ClapArg, App as ClapApp};

use std::path::Path;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use crate::accounting::*;
use crate::failed::*;
use crate::controls::*;
use crate::config::*;

const CHANNEL_SIZE: usize = 16;
const CHANNEL_LOW_WATERMARK: usize = 4;
const CHANNEL_HIGH_WATERMARK: usize = 8;


#[derive(Debug)]
pub enum ConstructionError {
    UnallocatedProgramInOutError(ProgramInOut),
    MissingSinkOutput(String),
    MissingTapInput(String),
}

impl std::fmt::Display for ConstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ConstructionError::UnallocatedProgramInOutError(e) => {
                write!(f, "ConstructionError: UnallocatedProgramInOutError: You specified the following input / output but it could not be allocated: {:?}", e)
            },
            ConstructionError::MissingTapInput(e) => {
                write!(f, "ConstructionError::MissingTapInput: The configuration requires the following inputs but they were not specified: {:?}", e)
            },
            ConstructionError::MissingSinkOutput(e) => {
                write!(f, "ConstructionError::MissingSinkOutput: The configuration requires the following outputs but they were not specified: {:?}", e)
            },
        }
    }
}


fn strip_ports_from_graph(graph: &mut TheGraph, current: petgraph::graph::NodeIndex) {

    fn strip_ports_from_graph_iter(graph: &mut TheGraph, current: petgraph::graph::NodeIndex) -> Vec<petgraph::graph::NodeIndex>  {

        let (incoming, outgoing): (Vec<petgraph::graph::NodeIndex>, Vec<petgraph::graph::NodeIndex>) = match graph[current].get(0..1) {
            Some("P") => {
                (
                    graph.neighbors_directed(current, Direction::Incoming).collect(),
                    graph.neighbors(current).collect()
                )
            }
            Some(_) => {
                ( vec![], graph.neighbors(current).collect() )
            }
            None => {
                panic!("strip_ports_from_graph({:?}) current could not be found in graph", current);
            }
        };

        if incoming.len() == 0 {
            return outgoing;
        }

        for i in &incoming {
            for o in &outgoing {
                graph.add_edge(*i, *o, 0);
            }
        }

        graph.remove_node(current);

        outgoing
    }

    let mut todo: Vec<petgraph::graph::NodeIndex> = strip_ports_from_graph_iter(graph, current);

    let mut i = 0;
    while i < todo.len() {
        let t = todo[i];
        let to_add = strip_ports_from_graph_iter(graph, t);
        for ta in to_add {
            if !todo.contains(&ta) {
                todo.push(ta);
            }
        }
        i = i + 1;
    }

}


#[test]
fn test_strip_ports_from_graph() {

    let mut graph = petgraph::stable_graph::StableGraph::<String, u32, petgraph::Directed, u32>::new();
    let faucet = graph.add_node("C#T#FAUCET".to_owned());
    let faucet_port_out = graph.add_node("P#T#FAUCET#O".to_owned());
    let buffer_port_in = graph.add_node("P#B#BUFFER_0#I".to_owned());
    let buffer = graph.add_node("C#B#BUFFER_0".to_owned());
    let buffer_port_out = graph.add_node("P#B#BUFFER_0#O".to_owned());
    let cmd_port_in = graph.add_node("P#C#CMD#I".to_owned());
    let cmd = graph.add_node("C#C#CMD".to_owned());

    graph.extend_with_edges(&[
        (faucet, faucet_port_out, 2),
        (faucet_port_out, buffer_port_in, 3),
        (buffer_port_in, buffer, 4),
        (buffer, buffer_port_out, 5),
        (buffer_port_out, cmd_port_in, 6),
        (cmd_port_in, cmd, 7),
    ]);

    strip_ports_from_graph(&mut graph, faucet);
    println!("{}", Dot::new(&graph));
    assert_eq!((3, 2), (graph.node_count(), graph.edge_count()));

}


fn find_graph_taps(graph: &TheGraph) -> Vec<petgraph::graph::NodeIndex> {
    let mut r: Vec<petgraph::graph::NodeIndex> = vec![];
    for ni in graph.node_indices() {
        if graph.neighbors_directed(ni, Direction::Incoming).count() == 0 {
            r.push(ni);
        }
    }

    r
}

#[test]
fn test_find_graph_taps() {

    use petgraph::stable_graph::StableGraph;
    let mut graph = StableGraph::<String, u32, petgraph::Directed, u32>::new();
    let faucet = graph.add_node("C#T#FAUCETo".to_owned());
    let faucet_port_out = graph.add_node("P#T#FAUCET#O".to_owned());
    let buffer_port_in = graph.add_node("P#B#BUFFER_0#I".to_owned());
    let buffer = graph.add_node("C#B#BUFFER_0".to_owned());
    let buffer_port_out = graph.add_node("P#B#BUFFER_0#O".to_owned());
    let cmd_port_in = graph.add_node("P#C#CMD#I".to_owned());
    let cmd = graph.add_node("C#C#CMD".to_owned());

    graph.extend_with_edges(&[
        (faucet, faucet_port_out, 2),
        (faucet_port_out, buffer_port_in, 3),
        (buffer_port_in, buffer, 4),
        (buffer, buffer_port_out, 5),
        (buffer_port_out, cmd_port_in, 6),
        (cmd_port_in, cmd, 7),
    ]);

    assert_eq!(vec![faucet], find_graph_taps(&graph));

}


#[derive(Debug)]
enum TapMethod {
    STDIN,
    FILENAME(String)
}

#[derive(Debug)]
enum SinkMethod {
    STDOUT,
    STDERR,
    FILENAME(String)
}


#[derive(Debug)]
struct Opts {
    debug: u64,
    pipeline: String,
    tap: HashMap<String, TapMethod>,
    sink: HashMap<String, SinkMethod>,
    graph: bool,
}

fn get_opts() -> Opts {

    let matches = ClapApp::new("pipeawesome")
        .version("0.0.0")
        .author("Matthew Forrester")
        .about("Create wierd and wonderful pipe systems")
        .arg(ClapArg::with_name("graph")
            .help("Prints out the generated graph and exits")
            .required(false)
            .long("graph")
        )
        .arg(ClapArg::with_name("pipeline")
            .help("Location pipeline specification file")
            .required(true)
            .takes_value(true)
            .short("p")
            .long("pipeline")
            .value_name("PIPEAWESOME_PIPELINE_SPECIFICATION")
            .env("PIPEAWESOME_PIPELINE_SPECIFICATION")
        )
        .arg(ClapArg::with_name("tap")
            .help("Where data comes from")
            .takes_value(true)
            .short("t")
            .long("tap")
            .multiple(true)
            .value_name("TAP")
        )
        .arg(ClapArg::with_name("sink")
            .help("Where data goes to")
            .takes_value(true)
            .short("s")
            .long("sink")
            .multiple(true)
            .value_name("SINK")
        )
        .arg(ClapArg::with_name("debug")
            .short("v")
            .multiple(true)
            .help("Sets the level of debug")
        ).get_matches();

    Opts {
        graph: matches.is_present("graph"),
        debug: matches.occurrences_of("debug"),
        tap: match matches.values_of("tap") {
            None => {
                HashMap::new()
            },
            Some(vs) => {
                let mut hm: HashMap<String, TapMethod> = HashMap::new();
                for v in vs {
                    let chunks:Vec<_> = v.splitn(2, "=").collect();
                    match chunks.as_slice() {
                        [k, "-"] => hm.insert(k.to_string(), TapMethod::STDIN),
                        [k, v] => hm.insert(k.to_string(), TapMethod::FILENAME(v.to_string())),
                        [_x] => None,
                        _ => None,
                    };
                }
                hm
            }
        },
        pipeline: match matches.value_of("pipeline") {
            None => "".to_owned(),
            Some(s) => s.to_string(),
        },
        sink: match matches.values_of("sink") {
            None => {
                HashMap::new()
            },
            Some(vs) => {
                let mut hm: HashMap<String, SinkMethod> = HashMap::new();
                for v in vs {
                    let chunks:Vec<_> = v.splitn(2, "=").collect();
                    match chunks.as_slice() {
                        [k, "-"] => hm.insert(k.to_string(), SinkMethod::STDOUT),
                        [k, "_"] => hm.insert(k.to_string(), SinkMethod::STDERR),
                        [k, v] => hm.insert(k.to_string(), SinkMethod::FILENAME(v.to_string())),
                        [_x] => None,
                        _ => None,
                    };
                }
                hm
            }
        }
    }

}


fn fix_config(filename: &Path) -> JSONConfig {

    let json = match std::fs::read_to_string(filename) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not read file '{}'\nError: {}", filename.to_string_lossy(), e);
            std::process::exit(1);
        }
    };

    let jc: JSONConfig = match serde_json::from_str(&json) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Could JSON decode file '{}'.\n\nThe JSON error is '{}'", filename.to_string_lossy(), e);
            std::process::exit(1);
        }
    };

    jc
}


struct Controls {
    tap_file: HashMap<String, Tap<FileGetRead<String>>>,
    tap_stdin: HashMap<String, Tap<StdinGetRead>>,
    junctions: HashMap<String, Junction>,
    buffers: HashMap<String, Buffer>,
    commands: HashMap<String, Command<HashMap<String, String>, Vec<String>, String, String, String, String>>,
    sink_stdout: HashMap<String, Sink<StdoutGetWrite>>,
    sink_stderr: HashMap<String, Sink<StderrGetWrite>>,
    sink_file: HashMap<String, Sink<FileGetWrite<String>>>,
}

enum JoinError{
    Destination(SpecType, ControlId),
    Source(SpecType, ControlId, Port),
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            JoinError::Destination(s, c) => {
                write!(f, "JoinError: Destination: {:?}:{:?}", s, c)
            }
            JoinError::Source(s, c, p) => {
                write!(f, "JoinError: Source: {:?}:{:?}:{:?}", s, c, p)
            }
        }
    }
}


impl Controls {

    fn get_processors(&mut self) -> HashMap<(SpecType, ControlId), Box<dyn Processable + Send>> {
        let mut r: HashMap<(SpecType, ControlId), Box<dyn Processable + Send>> = HashMap::new();

        for (k, mut v) in self.tap_file.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::TapSpec, k), Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.tap_stdin.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::TapSpec, k), Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.junctions.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::JunctionSpec, k), Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.buffers.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::BufferSpec, k), Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.commands.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::CommandSpec, k), Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.sink_stdout.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::SinkSpec, k), Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.sink_stderr.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::SinkSpec, k), Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.sink_file.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert((SpecType::SinkSpec, k), Box::new(p)); }
                None => (),
            }
        }

        r
    }

    fn get_output_rx(&mut self, spec_type: SpecType, name: &str, port: &Port) -> Option<Connected> {
        let src: &mut dyn InputOutput = self.get_mut(spec_type, name)?;
        src.get_output(port, CHANNEL_SIZE).ok()
    }


    fn set_input(&mut self, spec_type: SpecType, name: &str, priority: u32, input: Receiver<Line>) -> Option<ConnectionId> {
        let dst: &mut dyn InputOutput = self.get_mut(spec_type, name)?;
        dst.add_input(priority, input).ok()
    }


    fn join(&mut self, j: &JoinSpec) -> Result<(ConnectionId, ConnectionId), JoinError> {
        match self.get_output_rx(j.src.spec_type, &j.src.name, &j.src.port) {
            Some(connected) => {
                match self.set_input(j.dst.spec_type, &j.dst.name, j.priority, connected.0) {
                    Some(c2) => Ok((connected.1, c2)),
                    None => Err(JoinError::Destination(j.dst.spec_type, j.dst.name.to_owned()))
                }
            },
            None => Err(JoinError::Source(j.src.spec_type, j.src.name.to_owned(), j.src.port)),
        }
    }


    fn get_mut(&mut self, spec_type: SpecType, name: &str) -> Option<&mut dyn InputOutput> {
        match (spec_type, self.tap_file.get_mut(name)) {
            (SpecType::TapSpec, Some(t)) => { return Some(t); }
            _ => (),
        }
        match (spec_type, self.tap_stdin.get_mut(name)) {
            (SpecType::TapSpec, Some(t)) => { return Some(t); }
            _ => (),
        }
        match (spec_type, self.junctions.get_mut(name)) {
            (SpecType::JunctionSpec, Some(j)) => { return Some(j); }
            _ => (),
        }
        match (spec_type, self.buffers.get_mut(name)) {
            (SpecType::BufferSpec, Some(b)) => { return Some(b); }
            _ => (),
        }
        match (spec_type, self.commands.get_mut(name)) {
            (SpecType::CommandSpec, Some(c)) => { return Some(c); }
            _ => (),
        }
        match (spec_type, self.sink_stdout.get_mut(name)) {
            (SpecType::SinkSpec, Some(s)) => { return Some(s); }
            _ => (),
        }
        match (spec_type, self.sink_stderr.get_mut(name)) {
            (SpecType::SinkSpec, Some(s)) => { return Some(s); }
            _ => (),
        }
        match (spec_type, self.sink_file.get_mut(name)) {
            (SpecType::SinkSpec, Some(s)) => { return Some(s); }
            _ => (),
        }
        None
    }
}

struct StdinGetRead {}

impl GetRead for StdinGetRead {
    type R = std::io::Stdin;
    fn get_read(&self) -> std::io::Stdin {
        std::io::stdin()
    }
}

struct FileGetRead<P> where P: AsRef<std::path::Path> {
    p: P,
}

impl <P> GetRead for FileGetRead<P> where P: AsRef<std::path::Path> {
    type R = std::fs::File;
    fn get_read(&self) -> std::fs::File {
        std::fs::File::open(&self.p).unwrap()
    }
}

struct StdoutGetWrite {}
impl GetWrite for StdoutGetWrite {
    type W = std::io::Stdout;
    fn get_write(&self) -> std::io::Stdout {
        std::io::stdout()
    }
}

struct StderrGetWrite {}
impl GetWrite for StderrGetWrite {
    type W = std::io::Stderr;
    fn get_write(&self) -> std::io::Stderr {
        std::io::stderr()
    }
}

struct FileGetWrite<P> where P: AsRef<std::path::Path> {
    p: P,
}

impl <P> GetWrite for FileGetWrite<P> where P: AsRef<std::path::Path> {
    type W = std::fs::File;
    fn get_write(&self) -> std::fs::File {
        std::fs::File::create(&self.p).unwrap()
    }
}

#[derive(Debug)]
pub struct ProgramInOut {
    taps: HashMap<String, TapMethod>,
    sinks: HashMap<String, SinkMethod>,
}

fn construct(mut program_in_out: ProgramInOut, builders: &mut Vec<Builder<JSONLaunchSpec>>) -> Result<Controls, ConstructionError> {

    let mut controls: Controls = Controls {
        tap_file: HashMap::new(),
        tap_stdin: HashMap::new(),
        junctions: HashMap::new(),
        buffers: HashMap::new(),
        commands: HashMap::new(),
        sink_stdout: HashMap::new(),
        sink_stderr: HashMap::new(),
        sink_file: HashMap::new(),
    };

    for i in (0..builders.len()).rev() {
        let builder = builders.remove(i);
        let re_add = match builder {
            Builder::TapSpec(t) => {
                match program_in_out.taps.remove(&t.name) {
                    Some(TapMethod::FILENAME(s)) => {
                        controls.tap_file.insert(
                            t.name,
                            Tap::new(FileGetRead { p: s })
                        );
                        None
                    },
                    Some(TapMethod::STDIN) => {
                        controls.tap_stdin.insert(
                            t.name,
                            Tap::new(StdinGetRead {})
                        );
                        None
                    },
                    None => Some(Builder::TapSpec(t)),
                }
            }
            Builder::JunctionSpec(b) => {
                controls.junctions.insert(b.name.to_owned(), Junction::new());
                None
            },
            Builder::BufferSpec(b) => {
                controls.buffers.insert(b.name.to_owned(), Buffer::new());
                None
            },
            Builder::CommandSpec(c) => {
                controls.commands.insert(
                    c.name.to_owned(),
                    Command::new(
                        c.spec.command,
                        match c.spec.path {
                            None => ".".to_owned(),
                            Some(p) => p,
                        },
                        match c.spec.env {
                            None => HashMap::new(),
                            Some(h) => h,
                        },
                        match c.spec.args {
                            None => vec![],
                            Some(h) => h,
                        }
                    )
                );
                None
            },
            Builder::SinkSpec(s) => {
                match program_in_out.sinks.remove(&s.name) {
                    None => Some(Builder::SinkSpec(s)),
                    Some(SinkMethod::STDERR) => {
                        controls.sink_stderr.insert(
                            s.name.to_owned(),
                            Sink::new(StderrGetWrite {})
                        );
                        None
                    },
                    Some(SinkMethod::STDOUT) => {
                        controls.sink_stdout.insert(
                            s.name.to_owned(),
                            Sink::new(StdoutGetWrite {})
                        );
                        None
                    },
                    Some(SinkMethod::FILENAME(filename)) => {
                        controls.sink_file.insert(
                            s.name.to_owned(),
                            Sink::new(FileGetWrite { p: filename })
                        );
                        None
                    }
                }
            },
            Builder::JoinSpec(j) => Some(Builder::JoinSpec(j)),
        };
        match re_add {
            Some(x) => { builders.push(x); },
            None => (),
        }
    }

    for b in builders {
        match b {
            Builder::TapSpec(t) => {
                return Err(ConstructionError::MissingTapInput(t.name.clone()));
            }
            Builder::SinkSpec(t) => {
                return Err(ConstructionError::MissingSinkOutput(t.name.clone()));
            }
            _ => (),
        }
    }

    if (program_in_out.sinks.len() > 0) || (program_in_out.taps.len() > 0) {
        return Err(ConstructionError::UnallocatedProgramInOutError(program_in_out));
    }

    Ok(controls)

}

fn main() {

    let opts = get_opts();

    if opts.debug > 0 {
        eprintln!("OPTS: {:?}", opts);
    }

    let json_config = fix_config(Path::new(&opts.pipeline));
    let config = json_config.to_config();

    let mut identified = match identify(config.commands, config.outputs) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    if opts.graph {
        let taps = find_graph_taps(&identified.graph);
        for t in taps {
            strip_ports_from_graph(&mut identified.graph, t);
        }
        println!("{}", Dot::new(&identified.graph));
        std::process::exit(0);
    }

    let mut builders: Vec<Builder<JSONLaunchSpec>> = identified.spec.into_iter().collect();
    let program_in_out = ProgramInOut { taps: opts.tap, sinks: opts.sink };
    let mut controls = match construct(program_in_out, &mut builders) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error constructing controls: \n{}", e);
            std::process::exit(1);
        }
    };

    let mut accounting_builder = AccountingBuilder::new(
        CHANNEL_SIZE,
        CHANNEL_HIGH_WATERMARK,
        CHANNEL_LOW_WATERMARK
    );

    // println!("B: {:?}", builders);
    for b in builders {
        match b {
            Builder::JoinSpec(j) => {
                match controls.join(&j) {
                    Ok((c1, c2)) => {
                        accounting_builder.add_join(
                            ControlIO(j.src.spec_type, j.src.name, c1),
                            ControlIO(j.dst.spec_type, j.dst.name, c2)
                        );
                    },
                    Err(e) => { panic!("{}", &e); }
                }
            }
            _ => (),
        };
    }

    let option_accounting = accounting_builder.build();
    let raw_processors = controls.get_processors();

    if option_accounting.is_none() {
        eprintln!("Could not build proper accounting structures...");
        std::process::exit(1);
    }

    let mut accounting = option_accounting.unwrap();
    let mut processors = match accounting.convert_processors(raw_processors) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    struct CaughtProcessError {
        process_error: ProcessError,
        spec_type: Option<SpecType>,
        control_id: Option<ControlId>,
    }

    let jjoin_handle: JoinHandle<Result<usize, CaughtProcessError>> = std::thread::spawn(move || {

        let mut loop_number: usize = 0;
        let starts_and_ends: HashSet<ControlIndex> = accounting.get_ends()
            .into_iter()
            .fold(
                accounting.get_ends().into_iter().collect::<HashSet<ControlIndex>>(),
                |mut acc, item| {
                    acc.insert(item);
                    acc
                }
            );

        print!("CSV: {}", accounting.debug_header());

        let mut currents: Vec<Option<(ProcessCount, ControlIndex)>> = accounting.get_starts()
            .into_iter()
            .map(|tap_index| {
                Some((CHANNEL_HIGH_WATERMARK, tap_index))
            })
            .collect();

        let mut failed: Failed = Failed::new(100);
        loop {
            loop_number = loop_number + 1;

            // println!("CURRENTS: {:?}", currents);
            // println!("PIPE:     {:?}", &accounting.pipe_size);
            for current in &mut currents {
                match current {
                    Some((count, control_index)) => {

                        let next_status = match processors.get_mut(control_index) {
                            None => None,
                            Some(processor) => {
                                // TODO: Remove unwrap
                                // println!("C: {:?}", accounting.get_control(*control_index));
                                let process_status = match processor.process(*count) {
                                    Ok(ps) => ps,
                                    Err(e) => {
                                        let control = accounting.get_control(*control_index);
                                        return Err(CaughtProcessError {
                                            process_error: e,
                                            spec_type: control.map(|c| c.0),
                                            control_id: control.map(|c| c.1.to_owned()),
                                        })
                                    }
                                };
                                // let control = accounting.get_control(*control_index);
                                // println!("{:?} - {:?}", control, &process_status);
                                // std::thread::sleep(Duration::from_millis(100));
                                let csv_lines = accounting.update(
                                    &control_index,
                                    &process_status
                                );
                                // for line in csv_lines {
                                //     print!("CSV: {}", line);
                                // }
                                Accounting::update_failed(&control_index, &process_status, &mut failed);
                                let next_status = accounting.get_accounting_status(&control_index, process_status);
                                Some(next_status)
                            }
                        };

                        let rec = accounting.get_recommendation(next_status, &mut failed);

                        match rec {
                            Some((rec_read_count, rec_control_index)) => {
                                *count = rec_read_count;
                                *control_index = rec_control_index;
                                // *current = Some((read_count, (next_spec_type, next_control_id), tap_id));
                            }
                            None => {
                                // println!("===================");
                                // for f in accounting.get_finished() {
                                //     println!("FIN: {:?} {:?}", f, accounting.get_control(*f));
                                // }
                                std::thread::sleep(failed.till_clear());
                                // println!("ST: {:?}", starts_and_ends);
                                let should_exit = starts_and_ends
                                    .difference(&accounting.get_finished())
                                    .into_iter()
                                    .next()
                                    .is_none();
                                if should_exit {
                                    return Ok(0 as usize)
                                }
                            }
                        }


                    },
                    None => (),
                }
            };


        }

        // for (pair, proc) in &mut processors {
        //     let (spec_type, name) = pair;
        //     let process_result = proc.process();
        //     let accounting_return = accounting.update(spec_type.to_owned(), name.to_string(), process_result);
        //     // println!("PR: ({:?}, {:?}) = {:?}", spec_type, name, process_result);
        //     // println!("AC: {:?}", accounting.destinations);
        // }

        // if debug && ((loop_number % 10000) == 0) {
        // std::thread::sleep(std::time::Duration::from_millis(10));
        // }
        //     eprintln!("=============================================================== {}", loop_number);
        //     eprintln!("ACCOUNTING: SIZE: {:?}", accounting.pipe_size);
        //     // eprintln!("ACCOUNTING: INCOMING: {:?}", accounting.enter);
        //     // eprintln!("ACCOUNTING: OUTGOING: {:?}", accounting.leave);
        //     std::thread::sleep(std::time::Duration::from_millis(10));
        // println!("LOOP: {}", loop_number);

    });


    match jjoin_handle.join() {
        Err(join_error) => {
            eprintln!("Error joining threads: {:?}", join_error);
            std::process::exit(1);
        },
        Ok(joined) => {
            match joined {
                Err(e) => {
                        eprintln!(
                            "Control {:?}:{:?} encountered error {}",
                            e.spec_type,
                            e.control_id,
                            e.process_error
                        );
                    std::process::exit(1);
                }
                Ok(_) => (),
            }
        }
    }

}

