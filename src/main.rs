mod controls;
mod config;

#[path = "common_types.rs"]
mod common_types;

use petgraph::stable_graph::{ StableGraph, NodeIndex };
use petgraph::{ Direction };
use petgraph::dot::Dot;
use petgraph::algo::astar;
use clap::{Arg as ClapArg, App as ClapApp};

use csv::Writer;

use std::convert::TryFrom;
use std::path::Path;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, SyncSender, Receiver};
use std::cmp::{ Ord, Ordering };

use self::controls::*;
use self::config::*;
use self::common_types::Port;

const CHANNEL_SIZE: usize = 8192;


type ControlId = String;
#[derive(Hash, Debug, Clone)]
pub struct ControlInput(pub SpecType, pub ControlId, pub ConnectionId);
#[derive(Debug)]
pub struct AccountingMsg(SpecType, ControlId, ProcessStatus);

// impl ToOwned for ControlInput {
//     type Owned = ControlInput;

//     fn to_owned(&self) -> Self::Owned {
//         ControlInput(self.0, self.1.to_owned(), self.2)
//     }
// }

impl PartialEq for ControlInput {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0 && self.1 == other.1 && self.2 == other.2
    }
}
impl Eq for ControlInput {}

type PipeSizeHash = HashMap<SpecType, HashMap<ControlId, Vec<usize>>>;

#[derive(Debug)]
struct Accounting {
    controls: Vec<ControlInput>,
    channel_size: usize,
    channel_high_watermark: usize,
    channel_low_watermark: usize,
    sources: HashMap<ControlInput, ControlInput>,
    destinations: HashMap<ControlInput, ControlInput>,
    pipe_size: PipeSizeHash,
    enter: HashMap<ControlInput, usize>,
    leave: HashMap<ControlInput, usize>,
    finished: BTreeSet<ControlId>,
}

#[derive(PartialEq, Debug)]
enum AccountingOperation {
    Addition,
    Subtraction,
}

#[derive(PartialEq, Eq, PartialOrd)]
enum HungerLevel {
    Full = 0,
    Stuffed = 1,
    Satisfied = 2,
    Hungry = 3,
    Starved = 4,
}

struct AccountingReturn {
    csv_lines: Vec<String>,
    outbound_ports: Vec<(HungerLevel, usize)>,
    inbound_ports: Vec<(HungerLevel, usize)>,
    hunger_level: HungerLevel,
}

impl Accounting {
    fn new(channel_size: usize, channel_high_watermark: usize, channel_low_watermark: usize) -> Accounting {
        Accounting {
            controls: vec![],
            channel_size,
            channel_high_watermark,
            channel_low_watermark,
            sources: HashMap::new(),
            destinations: HashMap::new(),
            pipe_size: HashMap::new(),
            enter: HashMap::new(),
            leave: HashMap::new(),
            finished: BTreeSet::new(),
        }
    }

    fn update_stats(e_or_l: &mut HashMap<ControlInput, usize>, control_input: &ControlInput, count: &usize) {
        match e_or_l.get_mut(control_input) {
            None => { e_or_l.insert(control_input.to_owned(), *count); },
            Some(current) => { *current = *current + count },
        }
    }

    fn get_pipe_size(pipe_size: &PipeSizeHash, ci: &ControlInput) -> usize {

        let m = pipe_size.get(&ci.0).and_then(|hm| hm.get(&ci.1)).and_then(|v| v.get(ci.2));

        match m {
            Some(n) => *n,
            None => 0
        }

    }

    fn set_pipe_size(mut pipe_size: &mut PipeSizeHash, ci: &ControlInput, size: usize) {

        fn ensure_its_there<K: std::hash::Hash + Eq + Clone, V>(hm: &mut HashMap<K, V>, k: K, default_if_not_there: V) {

            match hm.get_mut(&k) {
                Some(_v) => (),
                None => {
                    hm.insert(k.clone(), default_if_not_there);
                }
            };

        }

        ensure_its_there(&mut pipe_size, ci.0, HashMap::new());

        let mut l2 = match pipe_size.get_mut(&ci.0) {
            Some(hm) => { hm },
            None => panic!("Accounting::set_pipe_size - get_mut after insert failed (1)"),
        };

        ensure_its_there(&mut l2, ci.1.clone(), Vec::new());

        match l2.get_mut(&ci.1) {
            Some(v) => {
                while v.len() <= ci.2 {
                    v.push(0);
                }
                v[ci.2] = size;
            }
            None => panic!("Accounting::set_pipe_size - get_mut after insert failed (2)"),
        }

    }

    fn update_pipe_size<'a>(mut pipe_size: &mut PipeSizeHash, control_input: &'a ControlInput, count: &usize, operation: AccountingOperation) -> Result<(), &'a ControlInput> {

        let get_count = |n: usize| {
            if operation == AccountingOperation::Addition {
                Some(n + *count) // i32::try_from(*count).unwrap()
            } else {
                if n >= *count {
                    Some(n - *count) // i32::try_from(*count).unwrap()
                } else {
                    None
                }
            }
        };

        match get_count(Accounting::get_pipe_size(&pipe_size, control_input)) {
            None => {
                Err(control_input)
            },
            Some(new_value) => {
                Accounting::set_pipe_size(&mut pipe_size, control_input, new_value);
                Ok(())
            },
        }

    }

    fn add_join(&mut self, src: ControlInput, dst: ControlInput) {
        Accounting::set_pipe_size(&mut self.pipe_size, &dst, 0);
        self.destinations.insert(src.clone(), dst.clone());
        self.sources.insert(dst.clone(), src.clone());
        if !self.controls.contains(&src) { self.controls.push(src); }
        if !self.controls.contains(&dst) { self.controls.push(dst); }
    }

    fn out_as_csv_line(out: &Vec<String>) -> Result<String, Box<dyn std::error::Error>> {
        let mut wtr = Writer::from_writer(vec![]);
        wtr.write_record(out)?;
        Ok(String::from_utf8(wtr.into_inner()?)?)
    }

    fn debug_header(&self) -> String {
        let out: Vec<String> = self.controls.iter().map(|c| format!("{:?}", c)).collect();
        match Accounting::out_as_csv_line(&out) {
            Ok(s) => {
                s
            },
            Err(e) => {
                panic!("PANIC!:CSVWRITER: {}", e);
            }
        }
    }

    fn debug_line(&self, control: &ControlInput, count: &usize, op: AccountingOperation) -> String {
        let mut out: Vec<String> = Vec::with_capacity(self.controls.len());
        for c in self.controls.iter() {
            let mut v: i64 = 0;
            if control == c {
                v = *count as i64;
                if op == AccountingOperation::Subtraction {
                    v = 0 - (*count as i64);
                }
            }
            out.push(format!("{}", v));
        }
        match Accounting::out_as_csv_line(&out) {
            Ok(s) => {
                s
            },
            Err(e) => {
                panic!("PANIC!:CSVWRITER: {}", e);
            }
        }
    }

    fn update(&mut self, spec_type: SpecType, control_id: ControlId, ps: ProcessStatus) -> Vec<String> {
        let mut r: Vec<String> = vec![];
        for (connection_id, count) in ps.wrote_to.iter() {
            let control = ControlInput(spec_type, control_id.to_owned(), *connection_id);
            r.push(self.debug_line(&control, count, AccountingOperation::Addition));
            Accounting::update_stats(&mut self.leave, &control, count);
            match self.destinations.get(&control) {
                Some(dst) => {
                    Accounting::update_pipe_size(
                        &mut self.pipe_size,
                        &dst,
                        count,
                        AccountingOperation::Addition
                    );
                },
                None => {
                    panic!("Accounting should have had a destination for {:?}, but did not", &control);
                }
            }
        }
        for (connection_id, count) in ps.read_from.iter() {
            let control = ControlInput(spec_type, control_id.to_owned(), *connection_id);
            r.push(self.debug_line(&control, count, AccountingOperation::Subtraction));
            Accounting::update_stats(&mut self.enter, &control, count);
            let r = Accounting::update_pipe_size(&mut self.pipe_size, &control, count, AccountingOperation::Subtraction);
            match r {
                Err(control_input) => {
                    panic!(
                        "Negative encountered during update_pipe_size: subraction(\n  pipe_size={:?},\n  control_input={:?},\n  count={:?}, process_status={:?})",
                        self.pipe_size,
                        control_input,
                        count,
                        ps
                    );
                }
                _ => (),
            }
        }
        match ps.stopped_by {
            StoppedBy::ExhaustedInput => {
                self.finished.insert(control_id);
            }
            _ => (),
        }

        r
    }

}


#[test]
fn test_accounting_buffers() {

    let mut accounting = Accounting::new(5, 4, 3);
    accounting.add_join(ControlInput(SpecType::TapSpec, "TAP1".to_owned(), 1), ControlInput(SpecType::BufferSpec, "BUF1".to_owned(), 0));
    accounting.add_join(ControlInput(SpecType::BufferSpec, "BUF1".to_owned(), 1), ControlInput(SpecType::CommandSpec, "CMD1".to_owned(), 0));

    let mut tap1_ps = ProcessStatus::new();
    let mut buf1_ps = ProcessStatus::new();
    let mut cmd1_ps = ProcessStatus::new();
    tap1_ps.add_to_wrote_to(vec![1; 9].into_iter());
    buf1_ps.add_to_read_from(vec![0; 7].into_iter());
    buf1_ps.add_to_wrote_to(vec![1; 3].into_iter());
    cmd1_ps.add_to_read_from(vec![0].into_iter());


    accounting.update(SpecType::TapSpec, "TAP1".to_owned(), tap1_ps);
    accounting.update(SpecType::BufferSpec, "BUF1".to_owned(), buf1_ps);
    accounting.update(SpecType::CommandSpec, "CMD1".to_owned(), cmd1_ps);

    enum ProcessAttempt {
        Sure(usize),
        NiceToHave(usize),
        BreakIfNotPossible(usize),
    }

    type ProcessPlan = (SpecType, ControlId, ProcessAttempt);

    let expected = vec![
        // TAP1 --(2)--> BUF1(4) --(2)--> CMD1(1)
        (SpecType::BufferSpec, "BUF1".to_owned(), ProcessAttempt::Sure(3)), // Get BUF1->CMD1 to be full
        // TAP1 --(2)--> BUF1(1) --(5)--> CMD1(1)
        (SpecType::TapSpec, "TAP1".to_owned(), ProcessAttempt::NiceToHave(2)), // TAP1->BUF1 below low water mark, take to high
        // TAP1 --(4)--> BUF1(1) --(5)--> CMD1(1)
        (SpecType::BufferSpec, "CMD1".to_owned(), ProcessAttempt::BreakIfNotPossible(1)), // Run CMD1 to clean way for BUF1 to be empty
        // TAP1 --(4)--> BUF1(1) --(4)--> CMD1(2)
        (SpecType::BufferSpec, "BUF1".to_owned(), ProcessAttempt::Sure(1)), // Make BUF1 empty
        // TAP1 --(4)--> BUF1(0) --(5)--> CMD1(2)
        (SpecType::BufferSpec, "CMD1".to_owned(), ProcessAttempt::NiceToHave(3)), // Get BUF1->CMD1 to below low water mark
        // TAP1 --(4)--> BUF1(0) --(2)--> CMD1(5)
        (SpecType::BufferSpec, "BUF1".to_owned(), ProcessAttempt::NiceToHave(3)), // Run buffer to try to get to BUF1->CMD1 to be full
        // TAP1 --(1)--> BUF1(0) --(5)--> CMD1(5)
        (SpecType::BufferSpec, "TAP1".to_owned(), ProcessAttempt::NiceToHave(3)), // Get TAP1->BUF1 to high water mark
        // TAP1 --(4)--> BUF1(0) --(5)--> CMD1(5)
        // Then run CMD1 -> BUF1 -> TAP1 etc
    ];

    // Every control has a status based on it's input channel. Given an
    // Accounting::new(10, 6, 3) it can be in the following states
    //
    //   * Full     = 10
    //   * Stuffed  = 7 - 10 (When any sources are)
    //   * Ok       = 3 - 6
    //   * Hungry   = 1 - 3 (When all sources are beneath)
    //   * Starved  = 0
    //
    // If any control is Full turn off (relevant) taps.
    // If more than a quarter of controls are Stuffed turn of (relevant) taps.
    // If all OK turn on (relevant) taps.
    // If more than a third Hungry turn on (relevant) taps.
    //
    // If Hungry and Source is Ok, run Source
    // If Stuffed and Destination Ok, run Self until Destination is Stuffed or Self Ok
    // If Full and not command or junction run



    // TODO: Desire for:
    //         * Below low mark to run till child hits high water mark (or full for buffers)
    //         * Commands after buffers which are are not empty to ran to empty buffer
    //         * Buffers to be empty
    //         * Commands after buffers which are are not empty to ran to low water mark
    //         * To be 1 below low water mark
    //         * To be empty

}



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

    let mut graph = StableGraph::<String, u32, petgraph::Directed, u32>::new();
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




// impl std::fmt::Display for Accounting {
//     fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
//         for e in enter {

//         // write!(f, "ConstructionError::MissingSinkOutput: The configuration requires the following outputs but they were not specified: {:?}", e)
//     }
// }


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


impl Controls {

    fn get_processors(&mut self) -> HashMap<(SpecType, String), Box<dyn Processable + Send>> {
        let mut r: HashMap<(SpecType, String), Box<dyn Processable + Send>> = HashMap::new();

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

    fn get_output_rx(&mut self, name: &str, port: &Port) -> Option<Connected> {
        let src: &mut dyn InputOutput = self.get_mut(name)?;
        src.get_output(port, CHANNEL_SIZE).ok()
    }


    fn set_input(&mut self, name: &str, priority: u32, input: Receiver<Line>) -> Option<ConnectionId> {
        let dst: &mut dyn InputOutput = self.get_mut(name)?;
        dst.add_input(priority, input).ok()
    }


    fn join(&mut self, j: &JoinSpec) -> Option<(ConnectionId, ConnectionId)> {
        match self.get_output_rx(&j.src.name, &j.src.port) {
            Some(connected) => {
                match self.set_input(&j.dst.name, j.priority, connected.0) {
                    Some(c2) => Some((connected.1, c2)),
                    None => None,
                }
            },
            None => None,
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
        match self.junctions.get_mut(name) {
            None => (),
            Some(j) => { return Some(j); }
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
            Builder::JoinSpec(j) => Some(Builder::JoinSpec(j)), // TODO
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
    let mut identified = identify(config.commands, config.outputs);
    if opts.graph {
        let taps = find_graph_taps(&identified.graph);
        for t in taps {
            strip_ports_from_graph(&mut identified.graph, t);
        }
        println!("{}", Dot::new(&identified.graph));
        std::process::exit(0);
    }
    println!("{}", Dot::new(&identified.graph));

    let mut builders: Vec<Builder<JSONLaunchSpec>> = identified.spec.into_iter().collect();
    let program_in_out = ProgramInOut { taps: opts.tap, sinks: opts.sink };
    let mut controls = match construct(program_in_out, &mut builders) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let mut accounting = Accounting::new(CHANNEL_SIZE, (CHANNEL_SIZE / 2) + 1, (CHANNEL_SIZE / 4) + 1);

    for b in builders {
        let r = match b {
            Builder::JoinSpec(j) => {
                match controls.join(&j) {
                    Some((c1, c2)) => {
                        accounting.add_join(
                            ControlInput(j.src.spec_type, j.src.name, c1),
                            ControlInput(j.dst.spec_type, j.dst.name, c2)
                        );
                        true
                    },
                    None => { panic!("Joining {:?} caused error", &j); }
                }
            }
            _ => false
        };
    }

    let mut processors = controls.get_processors();


    let jjoin_handle = std::thread::spawn(move || {

        fn do_sync_send(tx: &SyncSender<AccountingMsg>, msg: AccountingMsg) {

            enum E {
                Missing,
                CouldNotSend,
            }

            let mut om = Some(msg);
            let mut i = 0;
            while i < 100 {
                i = i + 1;
                let r = std::mem::take(&mut om)
                    .ok_or(E::Missing)
                    .and_then(|m| tx.send(m).map_err(|ee| match ee {
                        std::sync::mpsc::SendError(m) => {
                            std::mem::replace(&mut om, Some(m));
                            E::CouldNotSend
                        }
                    }));

                match r {
                    Ok(_) => return (),
                    Err(_) => (),
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("Could not send accounting message in 1s threshold");
        }

        let mut loop_number: usize = 0;
        let debug: bool = true;

        print!("CSV: {}", accounting.debug_header());
        loop {
            loop_number = loop_number + 1;

            for (pair, proc) in &mut processors {
                let (spec_type, name) = pair;
                let process_result = proc.process();
                // println!("PR: ({:?}, {:?}) = {:?}", spec_type, name, process_result);
                // println!("AC: {:?}", accounting);
                // for line in accounting.update(spec_type.to_owned(), name.to_string(), process_result) {
                //     print!("CSV: {}", line);
                // }
            }

            // if debug && ((loop_number % 10000) == 0) {
            //     eprintln!("=============================================================== {}", loop_number);
            //     eprintln!("ACCOUNTING: SIZE: {:?}", accounting.pipe_size);
            //     // eprintln!("ACCOUNTING: INCOMING: {:?}", accounting.enter);
            //     // eprintln!("ACCOUNTING: OUTGOING: {:?}", accounting.leave);
            //     std::thread::sleep(std::time::Duration::from_millis(10));
            // }
            // println!("LOOP: {}", loop_number);

        }
    });


    jjoin_handle.join().unwrap();

}

