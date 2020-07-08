use std::collections::HashMap;
use pipeawesome::{ Buffer, CommandOutput, Command, Pipe, Processable, Pullable, SingleInput, SingleOutput, Sink, Tap, Fan, Funnel };
use petgraph::graph::{ Graph, EdgeIndex, NodeIndex };
use petgraph::dot::Dot;


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


fn main() {

    // tap -> fan
    // fan -> buffer -> command -> pipe -> funnel
    // fan -> funnel
    // funnel -> sink

    let mut tap = Tap::new(|| { std::io::stdin() });
    let mut fan = Fan::new();
    let mut buffer = Buffer::new();
    let hm: HashMap<String, String> = HashMap::new();
    let mut command = Command::new("sed", ".", hm, vec!["s/^/ONE: /"]);
    let mut pipe = Pipe::new();
    let mut sink = Sink::new(|| { std::io::stdout() });
    let mut funnel = Funnel::new();

    fan.set_input(tap.get_output().unwrap());
    buffer.set_input(fan.add_output().unwrap());
    command.set_input(buffer.get_output().unwrap());
    pipe.set_input(command.get_output(CommandOutput::Stdout).unwrap());

    funnel.add_input(fan.add_output().unwrap());
    funnel.add_input(pipe.get_output().unwrap());

    sink.set_input(funnel.get_output().unwrap());

    let bs = buffer.get_buffer_size().unwrap();
    std::thread::spawn(move || {
        loop {
            bs.recv();
            // println!("BUFFER SIZE: {:?}", );
        }
    });

    std::thread::spawn(move || {
        loop {
            funnel.pull();
            fan.pull();
            pipe.pull();
        }
    });


    println!("COMMAND: {:?}", command.process());
    println!("BUFFER: {:?}", buffer.process());
    println!("TAP: {:?}", tap.process());

    for t in sink.process().unwrap() {
        t.join().unwrap();
    }
    // bs_thread.join().unwrap();

}

