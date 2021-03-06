use std::thread::JoinHandle;
use std::collections::HashSet;
use petgraph::{ Direction };
use petgraph::dot::Dot;
use clap::{Arg as ClapArg, App as ClapApp};

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use pipeawesome::accounting::*;
use pipeawesome::accounting_writer::*;
use pipeawesome::failed::*;
use pipeawesome::controls::*;
use pipeawesome::config::*;
use pipeawesome::stdin_out::{ FileGetRead, FileGetWrite, SinkMethod, TapMethod, StdinGetRead, StderrGetWrite, StdoutGetWrite };

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


fn strip_prequel_from_graph(graph: &mut TheGraph, substrs: Vec<&str>, current: petgraph::graph::NodeIndex) {

    fn strip_prequel_from_graph_iter(graph: &mut TheGraph, substr: &str, current: petgraph::graph::NodeIndex) -> (bool, Vec<petgraph::graph::NodeIndex>) {

        let (incoming, outgoing): (Vec<petgraph::graph::NodeIndex>, Vec<petgraph::graph::NodeIndex>) = match &graph[current].get(0..substr.len()) {
            Some(id) if id == &substr => {
                (
                    graph.neighbors_directed(current, Direction::Incoming).collect(),
                    graph.neighbors(current).collect()
                )
            }
            Some(_) => {
                ( vec![], graph.neighbors(current).collect() )
            }
            None => {
                panic!("strip_prequel_from_graph({:?}) current could not be found in graph", current);
            }
        };

        if incoming.is_empty() {
            return (false, outgoing);
        }

        for i in &incoming {
            for o in &outgoing {
                graph.add_edge(*i, *o, 0);
            }
        }

        graph.remove_node(current);

        (true, outgoing)
    }

    let mut todo: Vec<petgraph::graph::NodeIndex> = vec![];
    let mut removed = false;

    for s in &substrs {
        if !removed {
            let (have_removed, to_add) = strip_prequel_from_graph_iter(graph, s, current);
            removed = have_removed;
            for ta in to_add {
                if !todo.contains(&ta) {
                    todo.push(ta);
                }
            }
        }
    }

    removed = false;

    let mut i = 0;
    while i < todo.len() {
        let t = todo[i];
        for s in &substrs {
            if !removed {
                let (have_removed, to_add) = strip_prequel_from_graph_iter(graph, s, t);
                removed = have_removed;
                for ta in to_add {
                    if !todo.contains(&ta) {
                        todo.push(ta);
                    }
                }
            }
        }
        removed = false;
        i += 1;
    }

}


#[test]
fn test_strip_prequel_from_graph() {

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

    strip_prequel_from_graph(&mut graph, vec!["P", "C#B#"], faucet);
    println!("{}", Dot::new(&graph));
    assert_eq!((2, 1), (graph.node_count(), graph.edge_count()));

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


fn check_taps_can_go_everywhere(graph: &TheGraph) -> bool {

    for ni in graph.node_indices() {
        let mut found = false;
        for t in graph.externals(Direction::Incoming) {
            found = found || petgraph::algo::has_path_connecting(graph, t, ni, None);
            continue;
        }
        if found == false {
            return false;
        }
    }

    true

}


#[test]
fn test_check_taps_can_go_everywhere() {

    use petgraph::stable_graph::StableGraph;

    // With taps
    let mut g1 = StableGraph::<String, u32, petgraph::Directed, u32>::new();
    let g1_a = g1.add_node("A".to_owned());
    let g1_b = g1.add_node("B".to_owned());
    g1.extend_with_edges(&[
        (g1_a, g1_b, 2)
    ]);
    assert_eq!(true, check_taps_can_go_everywhere(&g1));

    // Just a cycle
    let mut g2 = StableGraph::<String, u32, petgraph::Directed, u32>::new();
    let g2_a = g2.add_node("A".to_owned());
    let g2_b = g2.add_node("B".to_owned());
    let g2_c = g2.add_node("C".to_owned());
    g2.extend_with_edges(&[
        (g2_a, g2_b, 2),
        (g2_b, g2_c, 2),
        (g2_c, g2_a, 2)
    ]);
    assert_eq!(false, check_taps_can_go_everywhere(&g2));

    // Two graphs
    let mut g3 = StableGraph::<String, u32, petgraph::Directed, u32>::new();
    let g3_a = g3.add_node("A".to_owned());
    let g3_b = g3.add_node("B".to_owned());
    let g3_c = g3.add_node("C".to_owned());
    let g3_d = g3.add_node("D".to_owned());
    g3.extend_with_edges(&[
        (g3_a, g3_b, 2),
        (g3_c, g3_d, 2),
    ]);
    assert_eq!(true, check_taps_can_go_everywhere(&g3));


}


#[derive(Debug)]
struct Opts {
    debug: u64,
    pipeline: String,
    tap: HashMap<String, TapMethod>,
    inline: bool,
    sink: HashMap<String, SinkMethod>,
    accounting: Option<SinkMethod>,
    graph: bool,
}

fn get_opts() -> Opts {

    let matches = ClapApp::new("pipeawesome")
        .version("0.0.0")
        .author("Matthew Forrester")
        .about("Create wierd and wonderful pipe systems")
        .arg(ClapArg::with_name("graph")
            .help("Prints out the generated graph and exits")
            .short("g")
            .required(false)
            .long("graph")
        )
        .arg(ClapArg::with_name("pipeline")
            .help("Location pipeline specification file")
            .required(true)
            .takes_value(true)
            .short("p")
            .long("pipeline")
            .env("PIPEAWESOME_PIPELINE_SPECIFICATION")
        )
        .arg(ClapArg::with_name("inline")
            .help("Treat the pipeline parameter as inline text as apposed to a filename")
            .short("n")
            .required(false)
            .long("inline")
        )
        .arg(ClapArg::with_name("input")
            .help("Where data comes from")
            .takes_value(true)
            .short("i")
            .long("input")
            .multiple(true)
        )
        .arg(ClapArg::with_name("output")
            .help("Where data goes to")
            .takes_value(true)
            .short("o")
            .long("output")
            .multiple(true)
        )
        .arg(ClapArg::with_name("accounting")
            .help("Where to write statistics about what is happening")
            .takes_value(true)
            .short("a")
            .long("accounting")
        )
        .arg(ClapArg::with_name("debug")
            .long("debug")
            .multiple(true)
            .help("Sets the level of debug")
        ).get_matches();

    Opts {
        graph: matches.is_present("graph"),
        inline: matches.is_present("inline"),
        debug: matches.occurrences_of("debug"),
        tap: match matches.values_of("input") {
            None => {
                HashMap::new()
            },
            Some(vs) => {
                let mut hm: HashMap<String, TapMethod> = HashMap::new();
                for v in vs {
                    let chunks:Vec<_> = v.splitn(2, '=').collect();
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
        sink: match matches.values_of("output") {
            None => {
                HashMap::new()
            },
            Some(vs) => {
                let mut hm: HashMap<String, SinkMethod> = HashMap::new();
                for v in vs {
                    let chunks:Vec<_> = v.splitn(2, '=').collect();
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
        },
        accounting: match matches.value_of("accounting") {
            Some("-") => Some(SinkMethod::STDOUT),
            Some("_") => Some(SinkMethod::STDERR),
            Some(x) if !x.is_empty() => Some(SinkMethod::FILENAME(x.to_string())),
            _ => None,
        }
    }

}


fn fix_config(mut opts: Opts) -> Opts {

    if opts.inline {
        return opts
    }

    opts.inline = true;
    opts.pipeline = match std::fs::read_to_string(&opts.pipeline) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not read file '{}'\nError: {}", &opts.pipeline, e);
            std::process::exit(1);
        }
    };


    opts
}

type CommandString = Command<HashMap<String, String>, Vec<String>, String, String, String, String>;

struct Controls {
    tap_file: HashMap<String, Tap<FileGetRead<String>>>,
    tap_stdin: HashMap<String, Tap<StdinGetRead>>,
    junctions: HashMap<String, Junction>,
    buffers: HashMap<String, Buffer>,
    commands: HashMap<String, CommandString>,
    sink_stdout: HashMap<String, Sink<StdoutGetWrite>>,
    sink_stderr: HashMap<String, Sink<StderrGetWrite>>,
    sink_file: HashMap<String, Sink<FileGetWrite<String>>>,
}

enum JoinError {
    Destination(SpecType, ControlId),
    Source(SpecType, ControlId, Port),
    SourcePortAlreadyTaken(SpecType, ControlId, Port),
    SourcePortNotOnControlType(SpecType, ControlId, Port),
    DestinationPortAlreadyTaken(SpecType, ControlId),
    DestinationPortNotOnControlType(SpecType, ControlId),
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            JoinError::Destination(s, c) => {
                write!(f, "JoinError: Destination: {:?}:{:?}", s, c)
            },
            JoinError::Source(s, c, p) => {
                write!(f, "JoinError: Source: {:?}:{:?}:{:?}", s, c, p)
            },
            JoinError::SourcePortNotOnControlType(s, c, p) => {
                write!(f, "JoinError: SourcePortNotOnControlType: {:?}:{:?}:{:?}", s, c, p)
            },
            JoinError::SourcePortAlreadyTaken(s, c, p) => {
                write!(f, "JoinError: SourcePortAlreadyTaken: {:?}:{:?}:{:?}", s, c, p)
            },
            JoinError::DestinationPortNotOnControlType(s, c) => {
                write!(f, "JoinError: DestinationPortNotOnControlType: {:?}:{:?}", s, c)
            },
            JoinError::DestinationPortAlreadyTaken(s, c) => {
                write!(f, "JoinError: DestinationPortAlreadyTaken: {:?}:{:?}", s, c)
            },
        }
    }
}


impl Controls {

    fn get_processors(&mut self) -> HashMap<(SpecType, ControlId), Box<dyn Processable + Send>> {

        let mut r: HashMap<(SpecType, ControlId), Box<dyn Processable + Send>> = HashMap::new();

        for (k, mut v) in self.tap_file.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::TapSpec, k), Box::new(p)); }
        }

        for (k, mut v) in self.tap_stdin.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::TapSpec, k), Box::new(p)); }
        }

        for (k, mut v) in self.junctions.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::JunctionSpec, k), Box::new(p)); }
        }

        for (k, mut v) in self.buffers.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::BufferSpec, k), Box::new(p)); }
        }

        for (k, mut v) in self.commands.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::CommandSpec, k), Box::new(p)); }
        }

        for (k, mut v) in self.sink_stdout.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::SinkSpec, k), Box::new(p)); }
        }

        for (k, mut v) in self.sink_stderr.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::SinkSpec, k), Box::new(p)); }
        }

        for (k, mut v) in self.sink_file.drain() {
            if let Ok(p) = v.get_processor() { r.insert((SpecType::SinkSpec, k), Box::new(p)); }
        }

        r
    }

    fn get_output_rx(&mut self, spec_type: SpecType, name: &str, port: &Port) -> Result<Option<Connected>, CannotAllocateOutputError> {
        let osrc: Option<&mut dyn InputOutput> = self.get_mut(spec_type, name);
        if let Some(src) = osrc {
            let c = src.get_output(port, CHANNEL_SIZE)?;
            return Ok(Some(c))
        }
        Ok(None)
    }


    fn set_input(&mut self, spec_type: SpecType, name: &str, priority: u32, input: Receiver<Line>) -> Result<Option<ConnectionId>,CannotAllocateInputError> {
        let m = self.get_mut(spec_type, name).map(
            |dst| dst.add_input(priority, input)
        );
        match m {
            Some(Ok(x)) => Ok(Some(x)),
            None => Ok(None),
            Some(Err(x)) => Err(x),
        }
    }


    fn join(&mut self, j: &JoinSpec) -> Result<(ConnectionId, ConnectionId), JoinError> {

        let connected = match self.get_output_rx(j.src.spec_type, &j.src.name, &j.src.port) {
            Ok(Some(connected)) => {
                Ok(connected)
            },
            Ok(None) => Err(JoinError::Source(j.src.spec_type, j.src.name.to_owned(), j.src.port)),
            Err(CannotAllocateOutputError::PortNotAvailableForControl) => Err(JoinError::SourcePortNotOnControlType(j.src.spec_type, j.src.name.to_owned(), j.src.port)),
            Err(CannotAllocateOutputError::PortAlreadyTaken) => Err(JoinError::SourcePortAlreadyTaken(j.src.spec_type, j.src.name.to_owned(), j.src.port)),
        }?;

        match self.set_input(j.dst.spec_type, &j.dst.name, j.priority, connected.0) {
            Ok(Some(c2)) => Ok((connected.1, c2)),
            Ok(None) => Err(JoinError::Destination(j.dst.spec_type, j.dst.name.to_owned())),
            Err(CannotAllocateInputError::PortNotAvailableForControl) => Err(
                JoinError::DestinationPortNotOnControlType(j.dst.spec_type, j.dst.name.to_owned())
            ),
            Err(CannotAllocateInputError::PortAlreadyTaken) => Err(
                JoinError::DestinationPortAlreadyTaken(j.dst.spec_type, j.dst.name.to_owned())
            ),
        }

    }


    fn get_mut(&mut self, spec_type: SpecType, name: &str) -> Option<&mut dyn InputOutput> {
        if let (SpecType::TapSpec, Some(t)) = (spec_type, self.tap_file.get_mut(name)) {
            return Some(t);
        }
        if let (SpecType::TapSpec, Some(t)) = (spec_type, self.tap_stdin.get_mut(name)) {
            return Some(t);
        }
        if let (SpecType::JunctionSpec, Some(j)) = (spec_type, self.junctions.get_mut(name)) {
            return Some(j);
        }
        if let (SpecType::BufferSpec, Some(b)) = (spec_type, self.buffers.get_mut(name)) {
            return Some(b);
        }
        if let (SpecType::CommandSpec, Some(c)) = (spec_type, self.commands.get_mut(name)) {
            return Some(c);
        }
        if let (SpecType::SinkSpec, Some(s)) = (spec_type, self.sink_stdout.get_mut(name)) {
            return Some(s);
        }
        if let (SpecType::SinkSpec, Some(s)) = (spec_type, self.sink_stderr.get_mut(name)) {
            return Some(s);
        }
        if let (SpecType::SinkSpec, Some(s)) = (spec_type, self.sink_file.get_mut(name)) {
            return Some(s);
        }
        None
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
                            Tap::new(FileGetRead::new(s))
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
                            Sink::new(FileGetWrite::new(filename))
                        );
                        None
                    }
                }
            },
            Builder::JoinSpec(j) => Some(Builder::JoinSpec(j)),
        };
        if let Some(x) = re_add {
            builders.push(x);
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

    if (!program_in_out.sinks.is_empty()) || (!program_in_out.taps.is_empty()) {
        return Err(ConstructionError::UnallocatedProgramInOutError(program_in_out));
    }

    Ok(controls)

}


    struct CaughtProcessError {
        process_error: ProcessError,
        spec_type: Option<SpecType>,
        control_id: Option<ControlId>,
    }

    enum WorkerError {
        CaughtProcessError(CaughtProcessError),
        AccountingWriterError(AccountingWriterError),
    }

    impl From<CaughtProcessError> for WorkerError {
        fn from(cpe: CaughtProcessError) -> WorkerError {
            WorkerError::CaughtProcessError(cpe)
        }
    }

    impl From<AccountingWriterError> for WorkerError {
        fn from(aw: AccountingWriterError) -> WorkerError {
            WorkerError::AccountingWriterError(aw)
        }
    }

fn main() {

    let opts = get_opts();

    if opts.debug > 0 {
        eprintln!("OPTS: {:?}", opts);
    }

    let opts = fix_config(opts);

    let json_config: JSONConfig = match serde_json::from_str(&opts.pipeline) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Could JSON decode pipeline.\n\nThe JSON error is '{}'", e);
            std::process::exit(1);
        }
    };

    let config = json_config.convert_to_config();

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
            strip_prequel_from_graph(&mut identified.graph, vec!["P", "C#B#", "C#J#"], t);
        }
        println!("{}", Dot::new(&identified.graph));
        std::process::exit(0);
    }

    if !check_taps_can_go_everywhere(&identified.graph) {
        eprintln!("Error because there are places that can not be reached by an input");
        std::process::exit(1);
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
    
    if builders.len() == 0 {
        eprintln!("NO BUILDERS!");
        std::process::exit(1);
    }

    let mut accounting_builder = AccountingBuilder::new(
        CHANNEL_SIZE,
        CHANNEL_HIGH_WATERMARK,
        CHANNEL_LOW_WATERMARK
    );

    let mut join_log: Vec<AccountingWriterJoinLogItem> = vec![];

    for b in builders {
        if let Builder::JoinSpec(j) = b {
            match controls.join(&j) {
                Ok((c1, c2)) => {
                    join_log.push(AccountingWriterJoinLogItem {
                        src_spec_type: j.src.spec_type,
                        src_control_id: j.src.name.clone(),
                        src_connection_id: c1,
                        dst_spec_type: j.dst.spec_type,
                        dst_control_id: j.dst.name.clone(),
                        dst_connection_id: c2,
                    });
                    accounting_builder.add_join(
                        ControlIO(j.src.spec_type, j.src.name, c1),
                        ControlIO(j.dst.spec_type, j.dst.name, c2)
                    );
                },
                Err(e) => {
                    let msg = match e {
                        JoinError::Destination(spec_type, control_id) => format!("Cannot find destination control {:?}:{:?}", spec_type, control_id),
                        JoinError::Source(spec_type, control_id, port) => format!("Cannot find source control {:?}:{:?} (port {:?})'", spec_type, control_id, port),
                        JoinError::SourcePortAlreadyTaken(spec_type, control_id, port) => format!("Port '{:?}' has already been allocated on {:?}:{:?}", port, spec_type, control_id),
                        JoinError::DestinationPortAlreadyTaken(spec_type, control_id) => format!("Port 'Input' has already been allocated on {:?}:{:?}", spec_type, control_id),

                        JoinError::SourcePortNotOnControlType(spec_type, control_id, port) => format!("Attempted to allocate port '{:?}' on {:?}:{:?} but it cannot exist on {:?}:{:?}", port, spec_type, control_id, spec_type, control_id),
                        JoinError::DestinationPortNotOnControlType(spec_type, control_id) => format!("Attempted to allocate port 'Input' on {:?}:{:?} but it cannot exist on {:?}:{:?}", spec_type, control_id, spec_type, control_id),
                    };
                    eprintln!("Error contructing pipes:\n {}", msg);
                    std::process::exit(1);
                }
            }
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

    let accounting_pref = opts.accounting;

    let jjoin_handle: JoinHandle<Result<usize, WorkerError>> = std::thread::spawn(move || {

        let mut accounting_writer = AccountingWriter::new(accounting_pref);

        for jl in join_log {
            accounting_writer.write_join(jl)?;
        }

        let starts_and_ends: HashSet<ControlIndex> = accounting.get_ends()
            .into_iter()
            .fold(
                accounting.get_ends().into_iter().collect::<HashSet<ControlIndex>>(),
                |mut acc, item| {
                    acc.insert(item);
                    acc
                }
            );

        let mut currents: Vec<Option<(ProcessCount, ControlIndex)>> = accounting.get_starts()
            .into_iter()
            .map(|tap_index| {
                Some((CHANNEL_HIGH_WATERMARK, tap_index))
            })
            .collect();

        let mut failed: Failed = Failed::new(100);
        accounting_writer.document_control_indices(&accounting, processors.iter().map(|(i, _)| *i).collect())?;
        loop {

            for current in &mut currents {

                let augment_with_processor: Option<(usize, usize, &mut Box<dyn Processable + std::marker::Send>)> = current
                    .and_then(|(count, control_index)| {
                        processors.get_mut(&control_index).map(|p| Some((count, control_index, p)))
                    })
                    .flatten();

                let next_status = match augment_with_processor {
                    Some((count, control_index, processor)) => {
                        let process_status: ProcessStatus = match processor.process(count) {
                            Ok(ps) => ps,
                            Err(e) => {
                                let control = accounting.get_control(control_index);
                                return Err(WorkerError::CaughtProcessError(CaughtProcessError {
                                    process_error: e,
                                    spec_type: control.map(|c| c.0),
                                    control_id: control.map(|c| c.1.to_owned()),
                                }));
                            }
                        };
                        accounting_writer.document_process_status(control_index, &process_status)?;
                        accounting.update(&control_index, &process_status);
                        Accounting::update_failed(&control_index, &process_status, &mut failed);
                        let next_status = accounting.get_accounting_status(&control_index, process_status);
                        Some(next_status)
                    },
                    None => None,
                };

                match (accounting.get_recommendation(next_status, &failed), current) {
                    (Some((rec_read_count, rec_control_index)), Some((count, control_index))) => {
                        *count = rec_read_count;
                        *control_index = rec_control_index;
                    },
                    _ => {
                        // println!("SE: {:?}\nAC: {:?}\n\n", &starts_and_ends.iter().map(|x| accounting.get_control(*x)).collect::<Vec<Option<&Control>>>(), &accounting.get_finished().iter().map(|x| accounting.get_control(*x)).collect::<Vec<Option<&Control>>>());
                        let should_exit = starts_and_ends
                            .difference(&accounting.get_finished())
                            .next()
                            .is_none();
                        if should_exit {
                            return Ok(0 as usize)
                        }
                        std::thread::sleep(failed.till_clear());
                    }
                };

            }
        }

    });


    match jjoin_handle.join() {
        Err(join_error) => {
            eprintln!("Error joining threads: {:?}", join_error);
            std::process::exit(1);
        },
        Ok(joined) => {
            match joined {
                Err(WorkerError::AccountingWriterError(e)) => {
                    eprintln!("Error writing accounting data {:?}", e);
                    std::process::exit(1);
                }
                Err(WorkerError::CaughtProcessError(e)) => {
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

