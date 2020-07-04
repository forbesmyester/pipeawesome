use std::collections::HashMap;
use pipeawesome::{ command, fan, funnel, sink, tap };
use petgraph::graph::Graph;
use petgraph::dot::Dot;


fn main() {


    let mut graph = Graph::<&str, u32>::new();
    let input = graph.add_node("INPUT");
    let pre = graph.add_node("PRE");
    let maths = graph.add_node("MATHS");
    let quality_control = graph.add_node("QUALITY_CONTROL");
    let fail_hot = graph.add_node("FAIL_HOT");
    let fail_cold = graph.add_node("FAIL_COLD");
    let just_right = graph.add_node("JUST_RIGHT");
    let out = graph.add_node("OUT");

    graph.extend_with_edges(&[
        (input, pre, 1),
        (pre, maths, 1),
        (maths, quality_control, 1),
        (quality_control, fail_cold, 1),
        (quality_control, fail_hot, 1),
        (quality_control, just_right, 1),
        (fail_hot, maths, 1),
        (fail_cold, maths, 1),
        (just_right, out, 1),
    ]);


    for n in graph.neighbors(quality_control) {
        println!("{:?}", graph[n]);
    }

    let t = tap(|| { std::io::stdin() });

    let mut f = fan(t, 3);
    let hm: HashMap<String, String> = HashMap::new();

    let (stdout, stderr) = command(f.remove(0), "sed", ".", hm, vec!["s/^/ONE: /"]).unwrap();
    f.push(stdout);
    f.push(stderr);
    let j = funnel(f);
    let s = sink(j, || std::io::stdout());

    std::thread::sleep(std::time::Duration::from_millis(1000))
}

