use std::collections::BTreeSet;
use std::collections::HashMap;
use self::controls::*;
use self::config::*;
use petgraph::graph::{ Graph, EdgeIndex, NodeIndex };
use std::sync::mpsc::{sync_channel, SyncSender, Receiver};
use petgraph::dot::Dot;

mod controls;
mod config;

const INPUT: &str = "I";
const COMMAND: &str = "C";
const FAN: &str = "A";
const OUTPUT: &str = "O";


#[derive(Debug)]
#[derive(PartialEq)]
struct GraphMoveProgress {
    finished: Vec<EdgeIndex>,
    in_progress: Vec<EdgeIndex>,
    steps: Vec<NodeIndex>,
}


fn graph_shuffle(graph: &Graph<&str, u32>, gmp: &mut GraphMoveProgress) {
    while gmp.in_progress.len() > 0 {
        let edge = gmp.in_progress.remove(0);
        let (_, node) = graph.edge_endpoints(edge).unwrap();
        let neighbors = graph.neighbors(node);
        let mut has_neighbors = false;
        for n in neighbors {
            has_neighbors = true;
            if graph[n][0..1] == *COMMAND {
                println!("F: {:?}", n);
                gmp.finished.push(graph.find_edge(node, n).unwrap());
            } else {
                println!("P: {:?}", n);
                gmp.in_progress.push(graph.find_edge(node, n).unwrap());
            }
        }
        if !has_neighbors {
            gmp.finished.push(edge);
        } else {
            gmp.steps.push(node);
        }
    }
}

#[test]
fn can_graph_shuffle() {

    let mut graph = Graph::<&str, u32, petgraph::Directed, u32>::new();
    let quality_control = graph.add_node("C:QUALITY_CONTROL");
    let fan_1 = graph.add_node("A:1");
    let fail_hot = graph.add_node("C:FAIL_HOT");
    let fail_cold = graph.add_node("C:FAIL_COLD");
    let fan_2 = graph.add_node("A:2");
    let funnel_1 = graph.add_node("U:1");
    let prepare_output = graph.add_node("O:prepare_output");

    graph.extend_with_edges(&[
        (quality_control, fan_1, 1),
        (fan_1, fail_cold, 1),
        (fan_1, fail_hot, 1),
        (fan_1, fan_2, 1),
        (fan_2, prepare_output, 1),
        (fail_hot, funnel_1, 1),
        (fail_cold, funnel_1, 1),
        (funnel_1, quality_control, 1),
    ]);

    let mut gmp = GraphMoveProgress {
        in_progress: vec![graph.find_edge(quality_control, fan_1).unwrap()],
        finished: vec![],
        steps: vec![],
    };

    let expected_gmp = GraphMoveProgress {
        in_progress: vec![],
        finished: vec![
            graph.find_edge(fan_1, fail_hot).unwrap(),
            graph.find_edge(fan_1, fail_cold).unwrap(),
            graph.find_edge(fan_2, prepare_output).unwrap(),
        ],
        steps: vec![
            fan_1,
            fan_2
        ]
    };

    graph_shuffle(&graph, &mut gmp);

    println!("S: {:?}", gmp.steps);
    assert_eq!(expected_gmp, gmp);
}


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


fn get_graph() -> Graph<String, u32> {
    let mut graph = Graph::<String, u32, petgraph::Directed, u32>::new();
    let tap = graph.add_node("TAP-OUT0".to_owned());
    let buf1i0 = graph.add_node("BUF1-IN0".to_owned());
    let buf1o1 = graph.add_node("BUF1-OUT1".to_owned());
    let buf1o2 = graph.add_node("BUF1-OUT2".to_owned());
    let buf2i0 = graph.add_node("BUF2-IN0".to_owned());
    let buf2i1 = graph.add_node("BUF2-IN1".to_owned());
    let buf2o2 = graph.add_node("BUF2-OUT2".to_owned());
    let cmdi0 = graph.add_node("CMD-IN0".to_owned());
    let cmdo1 = graph.add_node("CMD-OUT1".to_owned());
    let sink = graph.add_node("SINK-IN0".to_owned());

    graph.extend_with_edges(&[
        (tap, buf1i0, 0),
        (buf1o1, cmdi0, 0),
        (buf1o2, buf2i0, 0),
        (cmdo1, buf2i1, 0),
        (buf2o2, sink, 0),
    ]);

    graph
}


fn main() {

    // tap -> fan
    // fan -> buffer -> command -> pipe -> funnel
    // fan -> funnel
    // funnel -> sink

    let mut tap = Tap::new(|| { std::io::stdin() });
    let mut buffer_1 = Buffer::new();
    buffer_1.add_input(1, tap.get_output().unwrap());
    // let mut fan = Fan::new();
    // let mut buffer = Buffer::new();
    let hm: HashMap<String, String> = HashMap::new();
    let mut command = Command::new("sed", ".", hm, vec!["s/^/ONE: /"]);
    // let mut pipe = Pipe::new();
    // let mut funnel = Funnel::new();

    // fan.set_input(tap.get_output().unwrap());
    // buffer.set_input(fan.add_output().unwrap());
    command.set_input(buffer_1.add_output().unwrap()).unwrap();
    let mut buffer_2 = Buffer::new();
    buffer_2.add_input(1, command.get_output(CommandOutput::Stdout).unwrap()).unwrap();
    buffer_2.add_input(1, buffer_1.add_output().unwrap()).unwrap();

    let mut sink = Sink::new(|| { std::io::stdout() });
    sink.set_input(buffer_2.add_output().unwrap());

    let mut buffer_processor_1 = buffer_1.get_processor().unwrap();
    let mut buffer_processor_2 = buffer_2.get_processor().unwrap();
    let mut command_processor = command.get_processor().unwrap();

    impl PartialEq for ControlInput {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0 && self.1 == self.1
        }
    }
    impl Eq for ControlInput {}

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
                println!("ACCOUNTING: {:?}", accounting);
                return ();
            }
        }
    });

    let mut tap_processor = tap.get_processor().unwrap();

    let mut sink_processor = sink.get_processor().unwrap();

    let graph = get_graph();

    let join_handle = std::thread::spawn(move || {
        loop {



            // println!("BP1: {:?}", buffer_processor_1.process());
            // println!("BP2: {:?}", buffer_processor_2.process());
            acc_tx_buffer.send(AccountingMsg("TAP".to_owned(), tap_processor.process())).unwrap();
            acc_tx_buffer.send(AccountingMsg("BUF1".to_owned(), buffer_processor_1.process())).unwrap();
            acc_tx_buffer.send(AccountingMsg("BUF2".to_owned(), buffer_processor_2.process())).unwrap();
            acc_tx_buffer.send(AccountingMsg("CMD".to_owned(), command_processor.process())).unwrap();
            acc_tx_buffer.send(AccountingMsg("SINK".to_owned(), sink_processor.process())).unwrap();
            // std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    // println!("COMMAND: {:?}", command.process());
    // println!("BUFFER: {:?}", buffer.process());
    // println!("TAP: {:?}", tap.process());

    join_handle.join().unwrap();
    // for t in sink.process().unwrap() {
    //     t.join().unwrap();
    // }
    // // bs_thread.join().unwrap();

}

