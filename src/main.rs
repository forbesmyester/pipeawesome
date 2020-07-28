mod controls;
mod config;

#[path = "common_types.rs"]
mod common_types;

use std::collections::BTreeSet;
use std::collections::HashMap;
use self::controls::*;
use self::config::*;
use petgraph::graph::{ Graph, EdgeIndex, NodeIndex };
use std::sync::mpsc::{sync_channel, SyncSender, Receiver};
use petgraph::dot::Dot;
use self::common_types::Port;

// const INPUT: &str = "I";
// const COMMAND: &str = "C";
// const FAN: &str = "A";
// const OUTPUT: &str = "O";


// #[derive(Debug)]
// #[derive(PartialEq)]
// struct GraphMoveProgress {
//     finished: Vec<EdgeIndex>,
//     in_progress: Vec<EdgeIndex>,
//     steps: Vec<NodeIndex>,
// }


// fn graph_shuffle(graph: &Graph<&str, u32>, gmp: &mut GraphMoveProgress) {
//     while gmp.in_progress.len() > 0 {
//         let edge = gmp.in_progress.remove(0);
//         let (_, node) = graph.edge_endpoints(edge).unwrap();
//         let neighbors = graph.neighbors(node);
//         let mut has_neighbors = false;
//         for n in neighbors {
//             has_neighbors = true;
//             if graph[n][0..1] == *COMMAND {
//                 println!("F: {:?}", n);
//                 gmp.finished.push(graph.find_edge(node, n).unwrap());
//             } else {
//                 println!("P: {:?}", n);
//                 gmp.in_progress.push(graph.find_edge(node, n).unwrap());
//             }
//         }
//         if !has_neighbors {
//             gmp.finished.push(edge);
//         } else {
//             gmp.steps.push(node);
//         }
//     }
// }

// #[test]
// fn can_graph_shuffle() {

//     let mut graph = Graph::<&str, u32, petgraph::Directed, u32>::new();
//     let quality_control = graph.add_node("C:QUALITY_CONTROL");
//     let fan_1 = graph.add_node("A:1");
//     let fail_hot = graph.add_node("C:FAIL_HOT");
//     let fail_cold = graph.add_node("C:FAIL_COLD");
//     let fan_2 = graph.add_node("A:2");
//     let funnel_1 = graph.add_node("U:1");
//     let prepare_output = graph.add_node("O:prepare_output");

//     graph.extend_with_edges(&[
//         (quality_control, fan_1, 1),
//         (fan_1, fail_cold, 1),
//         (fan_1, fail_hot, 1),
//         (fan_1, fan_2, 1),
//         (fan_2, prepare_output, 1),
//         (fail_hot, funnel_1, 1),
//         (fail_cold, funnel_1, 1),
//         (funnel_1, quality_control, 1),
//     ]);

//     let mut gmp = GraphMoveProgress {
//         in_progress: vec![graph.find_edge(quality_control, fan_1).unwrap()],
//         finished: vec![],
//         steps: vec![],
//     };

//     let expected_gmp = GraphMoveProgress {
//         in_progress: vec![],
//         finished: vec![
//             graph.find_edge(fan_1, fail_hot).unwrap(),
//             graph.find_edge(fan_1, fail_cold).unwrap(),
//             graph.find_edge(fan_2, prepare_output).unwrap(),
//         ],
//         steps: vec![
//             fan_1,
//             fan_2
//         ]
//     };

//     graph_shuffle(&graph, &mut gmp);

//     println!("S: {:?}", gmp.steps);
//     assert_eq!(expected_gmp, gmp);
// }


// #[test]
// fn can_rescore() {

//     let mut graph = Graph::<&str, u32, petgraph::Directed, u32>::new();
//     let input = graph.add_node("I:IN");
//     let pre = graph.add_node("C:PRE");
//     let maths = graph.add_node("C:MATHS");
//     let quality_control = graph.add_node("C:QUALITY_CONTROL");
//     let fan_1 = graph.add_node("A:1");
//     let fail_hot = graph.add_node("C:FAIL_HOT");
//     let fail_cold = graph.add_node("C:FAIL_COLD");
//     let just_right = graph.add_node("C:JUST_RIGHT");
//     let out = graph.add_node("O:OUT");


//     graph.extend_with_edges(&[
//         (input, pre, 1),
//         (pre, maths, 1),
//         (maths, quality_control, 1),
//         (quality_control, fan_1, 1),
//         (fan_1, fail_cold, 1),
//         (fan_1, fail_hot, 1),
//         (fan_1, just_right, 1),
//         (fail_hot, maths, 1),
//         (fail_cold, maths, 1),
//         (just_right, out, 1),
//     ]);

//     assert_eq!(
//         graph.edge_weight(
//             graph.find_edge(fan_1, fail_hot).unwrap()
//         ).unwrap(),
//         &2
//     );

//     for n in graph.neighbors(quality_control) {
//         println!("{:?}", graph[n]);
//     }


// }

type ControlId = String;
type ConnectionId = usize;
#[derive(Hash, Debug)]
struct ControlInput(ControlId, ConnectionId);
struct AccountingMsg(ControlId, ProcessStatus);

impl PartialEq for ControlInput {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0 && self.1 == self.1
    }
}
impl Eq for ControlInput {}

#[derive(Debug)]
struct Accounting {
    enter: HashMap<ControlInput, usize>,
    leave: HashMap<ControlInput, usize>,
    finished: BTreeSet<ControlId>,
}

impl Accounting {
    fn new() -> Accounting {
        Accounting {
            enter: HashMap::new(),
            leave: HashMap::new(),
            finished: BTreeSet::new(),
        }
    }

    fn update_stats(e_or_l: &mut HashMap<ControlInput, usize>, control_input: ControlInput, count: &usize) {
        match e_or_l.get_mut(&control_input) {
            None => { e_or_l.insert(control_input, *count); },
            Some(current) => { *current = *current + count },
        }
    }

    fn update(&mut self, control_id: ControlId, ps: ProcessStatus) {
        for (connection_id, count) in ps.read_from.iter() {
            Accounting::update_stats(&mut self.enter, ControlInput(control_id.to_owned(), *connection_id), count);
        }
        for (connection_id, count) in ps.wrote_to.iter() {
            Accounting::update_stats(&mut self.leave, ControlInput(control_id.to_owned(), *connection_id), count);
        }
        match ps.stopped_by {
            StoppedBy::ExhaustedInput => {
                self.finished.insert(control_id);
            }
            _ => (),
        }
    }

}


#[derive(Debug)]
enum ProgramInOut {
    STDIN,
    STDOUT,
    STDERR,
    FILENAME(String)
}


#[derive(Debug)]
struct Opts {
    debug: u64,
    tap: HashMap<String, ProgramInOut>,
    sink: HashMap<String, ProgramInOut>,
}

// fn get_opts() -> Opts {

//     let matches = ClapApp::new("pipeawesome")
//         .version("0.0.0")
//         .author("Matthew Forrester")
//         .about("Create wierd and wonderful pipe systems")
//         .arg(ClapArg::with_name("tap")
//             .help("Where data comes from")
//             .takes_value(true)
//             .short("t")
//             .long("tap")
//             .multiple(true)
//             .value_name("TAP")
//         )
//         .arg(ClapArg::with_name("sink")
//             .help("Where data goes to")
//             .takes_value(true)
//             .short("s")
//             .long("sink")
//             .multiple(true)
//             .value_name("SINK")
//         )
//         .arg(ClapArg::with_name("debug")
//             .short("v")
//             .multiple(true)
//             .help("Sets the level of debug")
//         ).get_matches();

//     Opts {
//         debug: matches.occurrences_of("debug"),
//         stream: match matches.values_of("stream") {
//             None => {
//                 HashMap::new()
//             },
//             Some(vs) => {
//                 let mut hm: HashMap<String,String> = HashMap::new();
//                 for v in vs {
//                     let chunks:Vec<_> = v.splitn(2, "=").collect();
//                     match chunks.as_slice() {
//                         [k, v] => hm.insert(k.to_string(), v.to_string()),
//                         [_x] => None,
//                         _ => None,
//                     };
//                 }
//                 hm
//             }
//         }
//     }

// }


fn fix_config() -> Specification<JSONLaunchSpec> {

    let filename = "./pad_to_8.paspec.json";
    let json = match std::fs::read_to_string(filename) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not read file '{}'\nError: {}", filename, e);
            std::process::exit(1);
        }
    };

    let jc: JSONConfig = match serde_json::from_str(&json) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Could JSON decode file '{}'.\n\nThe JSON error is '{}'", filename, e);
            std::process::exit(1);
        }
    };

    let c = jc.to_config();

    identify(c.commands, c.outputs)

}


struct Controls {
    tap_file: HashMap<String, Tap<FileGetRead<String>>>,
    tap_stdin: HashMap<String, Tap<StdinGetRead>>,
    buffers: HashMap<String, Buffer>,
    commands: HashMap<String, Command<HashMap<String, String>, Vec<String>, String, String, String, String>>,
    sink_stdout: HashMap<String, Sink<StdoutGetWrite>>,
    sink_stderr: HashMap<String, Sink<StderrGetWrite>>,
    sink_file: HashMap<String, Sink<FileGetWrite<String>>>,
}


impl Controls {

    fn get_processors(&mut self) -> HashMap<String, Box<dyn Processable + Send>> {
        let mut r: HashMap<String, Box<dyn Processable + Send>> = HashMap::new();

        for (k, mut v) in self.tap_file.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert(k, Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.tap_stdin.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert(k, Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.buffers.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert(k, Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.commands.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert(k, Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.sink_stdout.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert(k, Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.sink_stderr.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert(k, Box::new(p)); }
                None => (),
            }
        }

        for (k, mut v) in self.sink_file.drain() {
            match v.get_processor().ok() {
                Some(p) => { r.insert(k, Box::new(p)); }
                None => (),
            }
        }

        r
    }

    fn get_output_rx(&mut self, name: &str, port: &Port) -> Option<Receiver<Line>> {

        let src: &mut dyn InputOutput = self.get_mut(name)?;

        match src.get_output(port, 8192).ok() {
            Some(tx) => {
                Some(tx)
            },
            _ => None
        }

    }


    fn set_input(&mut self, name: &str, input: Receiver<Line>) -> Option<()> {
        let dst: &mut dyn InputOutput = self.get_mut(name)?;
        dst.add_input(input).ok()
    }


    fn join(&mut self, j: JoinSpec) -> bool {
        match self.get_output_rx(&j.src.name, &j.src.port) {
            Some(rx) => {
                self.set_input(&j.dst.name, rx).is_some()
            },
            _ => false
        }
    }


    fn get_mut(&mut self, name: &str) -> Option<&mut dyn InputOutput> {
        match self.tap_file.get_mut(name) {
            None => (),
            Some(t) => { return Some(t); }
        }
        match self.tap_stdin.get_mut(name) {
            None => (),
            Some(t) => { return Some(t); }
        }
        match self.buffers.get_mut(name) {
            None => (),
            Some(b) => { return Some(b); }
        }
        match self.commands.get_mut(name) {
            None => (),
            Some(c) => { return Some(c); }
        }
        match self.sink_stdout.get_mut(name) {
            None => (),
            Some(s) => { return Some(s); }
        }
        match self.sink_stderr.get_mut(name) {
            None => (),
            Some(s) => { return Some(s); }
        }
        match self.sink_file.get_mut(name) {
            None => (),
            Some(s) => { return Some(s); }
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

fn construct(mut taps: HashMap<String, ProgramInOut>, mut sinks: HashMap<String, ProgramInOut>, builders: &mut Vec<Builder<JSONLaunchSpec>>) -> Controls {

    let mut controls: Controls = Controls {
        tap_file: HashMap::new(),
        tap_stdin: HashMap::new(),
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
                match taps.remove(&t.name) {
                    Some(ProgramInOut::FILENAME(s)) => {
                        controls.tap_file.insert(
                            t.name,
                            Tap::new(FileGetRead { p: s })
                        );
                        None
                    },
                    Some(ProgramInOut::STDIN) => {
                        controls.tap_stdin.insert(
                            t.name,
                            Tap::new(StdinGetRead {})
                        );
                        None
                    },
                    Some(ProgramInOut::STDOUT) => Some(Builder::TapSpec(t)),
                    Some(ProgramInOut::STDERR) => Some(Builder::TapSpec(t)),
                    None => Some(Builder::TapSpec(t)),
                }
            }
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
                match sinks.remove(&s.name) {
                    None => Some(Builder::SinkSpec(s)),
                    Some(ProgramInOut::STDIN) => Some(Builder::SinkSpec(s)),
                    Some(ProgramInOut::STDERR) => {
                        controls.sink_stderr.insert(
                            s.name.to_owned(),
                            Sink::new(StderrGetWrite {})
                        );
                        None
                    },
                    Some(ProgramInOut::STDOUT) => {
                        controls.sink_stdout.insert(
                            s.name.to_owned(),
                            Sink::new(StdoutGetWrite {})
                        );
                        None
                    },
                    Some(ProgramInOut::FILENAME(filename)) => {
                        controls.sink_file.insert(
                            s.name.to_owned(),
                            Sink::new(FileGetWrite { p: filename })
                        );
                        None
                    }
                }
            },
            Builder::JoinSpec(j) => Some(Builder::JoinSpec(j)), // TODO
        };
        match re_add {
            Some(x) => { builders.push(x); },
            None => (),
        }
    }

    controls

}

fn main() {

    let i = fix_config();
    let mut builders: Vec<Builder<JSONLaunchSpec>> = i.spec.into_iter().collect();
    let mut sinks: HashMap<String, ProgramInOut> = HashMap::new();
    sinks.insert("OUTPUT".to_owned(), ProgramInOut::STDOUT);
    let mut taps: HashMap<String, ProgramInOut> = HashMap::new();
    taps.insert("FAUCET".to_owned(), ProgramInOut::STDIN);
    let mut controls = construct(taps, sinks, &mut builders);

    for b in builders {
        println!("J: {:?}", b);
        let r = match b {
            Builder::JoinSpec(j) => {
                controls.join(j)
            }
            _ => false
        };
        println!("R: {}\n\n", r);
    }

    let mut processors = controls.get_processors();

    let (acc_tx_buffer, acc_rx): (SyncSender<AccountingMsg>, Receiver<AccountingMsg>) = sync_channel(1);

    let join_handle = std::thread::spawn(move || {
        let mut accounting = Accounting::new();
        loop {
            match acc_rx.recv() {
                Ok(AccountingMsg(control_id, process_status)) => {
                    accounting.update(control_id, process_status);
                },
                Err(e) => {
                    panic!("Should not be here: {:?}", e)
                },
            }
            if accounting.finished.len() == 5 {
                println!("ACCOUNTING: FINISHED: {:?}", accounting);
                std::thread::sleep(std::time::Duration::from_millis(2000));
                return ();
            }
            // println!("ACCOUNTING: ONGOING: {:?}\n", accounting);
        }
    });

    let jjoin_handle = std::thread::spawn(move || {
        loop {

            for (name, proc) in &mut processors {
                acc_tx_buffer.send(AccountingMsg(name.to_string(), proc.process())).unwrap();
            }

        }
    });


    join_handle.join().unwrap();

}

