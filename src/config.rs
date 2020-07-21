use std::ffi::{ OsStr, OsString };
use std::convert::From;
use std::path::Path;
use petgraph::csr::Neighbors;
use std::iter::IntoIterator;
use std::cmp::{ Ord, Ordering };
use std::collections::{ BTreeMap, HashMap, BTreeSet };
use petgraph::stable_graph::{ StableGraph, NodeIndex };
use petgraph::{ Direction };
use petgraph::dot::Dot;
use petgraph::visit::Bfs;

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord)]
enum Control {
    Command,
    Buffer,
    Sink,
    Tap,
}

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd)]
enum Port {
    OUT,
    IN,
    ERR,
}

impl std::fmt::Display for Port {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Port::IN => write!(f, "I"),
            Port::OUT => write!(f, "O"),
            Port::ERR => write!(f, "E"),
        }
    }
}

fn port_str_to_enum(s: &str) -> Port {
    if s == "I" {
        return Port::IN;
    }
    if s == "O" { Port::OUT } else { Port::ERR }
}

fn string_to_control(s: &str) -> Control {
    match s {
        "C" => Control::Command,
        "B" => Control::Buffer,
        "S" => Control::Sink,
        "T" => Control::Tap,
        _ => panic!("Don't know about control {}", s),
    }
}

fn control_to_string(s: &Control) -> String {
    match s {
        Control::Command => "C".to_owned(),
        Control::Buffer => "B".to_owned(),
        Control::Sink => "S".to_owned(),
        Control::Tap => "T".to_owned(),
    }
}

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd)]
struct Target {
    control: Control,
    name: String,
    port: Port,
}

impl Ord for Target {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.name.cmp(&other.name) {
            Ordering::Equal => {
                if self.port == other.port {
                    return Ordering::Equal;
                }
                if self.port < other.port { Ordering::Less } else { Ordering::Greater }
            },
            x => x,
        }
    }
}

type Outputs = BTreeMap<String, Vec<Target>>;

// #[derive(Debug, Eq, PartialOrd, Ord, PartialEq)]
// struct Specification();
#[derive(Debug, Eq, PartialOrd, Ord, PartialEq)]
pub struct Specification<E, P, O, A, K, V, R>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    command: O,
    path: P,
    env: Option<E>,
    args: Option<A>,
}

impl <E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>> Specification<E, P, O, A, K, V, R> {
    pub fn new(env: E, path: P, command: O, args: A) -> Specification<E, P, O, A, K, V, R> {
        Specification { command, path, env: Some(env), args: Some(args) }
    }
}


#[derive(Debug, PartialEq)]
struct Line<E, P, O, A, K, V, R>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    src: Vec<Target>,
    spec: Specification<E, P, O, A, K, V, R>,
    name: String,
}

#[derive(Debug)]
#[derive(PartialEq)]
struct BufferSpec {
    src: Vec<Target>,
    dst: Vec<Target>,
}

#[derive(Debug)]
#[derive(PartialEq)]
struct IdentifyNonCommands {
    taps: Vec<String>,
    sinks: Vec<String>,
    buffers: Vec<BufferSpec>,
}

#[derive(Debug, PartialEq, Eq, std::hash::Hash)]
struct DirectConnect {
    src: NodeIndex<u32>,
    dst: NodeIndex<u32>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Join {
    src: Target,
    dst: Target,
}

#[derive(Debug)]
struct Command<E, P, O, A, K, V, R>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    name: String,
    spec: Specification<E, P, O, A, K, V, R>,
}

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    PartialEq for Command<E, P, O, A, K, V, R>
{
    fn eq(&self, other: &Self) -> bool {
        self.name.cmp(&other.name) == Ordering::Equal
    }
}

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    Eq for Command<E, P, O, A, K, V, R> { }

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    Ord for Command<E, P, O, A, K, V, R>
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    PartialOrd for Command<E, P, O, A, K, V, R>
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Buffer { name: String }
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Sink { name: String }
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Tap { name: String }

#[derive(Debug)]
enum Builder<E, P, O, A, K, V, R>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    Join(Join),
    Command(Command<E, P, O, A, K, V, R>),
    Buffer(Buffer),
    Sink(Sink),
    Tap(Tap),
}

fn builder_enum_type_to_usize<E, P, O, A, K, V, R>(b: &Builder<E, P, O, A, K, V, R>) -> usize
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    match b {
        Builder::Join(_) => 1,
        Builder::Command(_) => 2,
        Builder::Buffer(_) => 3,
        Builder::Sink(_) => 4,
        Builder::Tap(_) => 5,
    }
}

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    PartialEq for Builder<E, P, O, A, K, V, R>
{
    fn eq(&self, other: &Self) -> bool {
        builder_enum_type_to_usize(self) == builder_enum_type_to_usize(other)
    }
}

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    Eq for Builder<E, P, O, A, K, V, R> { }

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    Ord for Builder<E, P, O, A, K, V, R>
{
    fn cmp(&self, other: &Self) -> Ordering {
        let s = builder_enum_type_to_usize(self);
        let o = builder_enum_type_to_usize(other);
        match (self, other) {
            (Builder::Join(a), Builder::Join(b)) => { a.cmp(b) },
            (Builder::Command(a), Builder::Command(b)) => { a.cmp(b) },
            (Builder::Buffer(a), Builder::Buffer(b)) => { a.cmp(b) },
            (Builder::Sink(a), Builder::Sink(b)) => { a.cmp(b) },
            (Builder::Tap(a), Builder::Tap(b)) => { a.cmp(b) },
            (a, b) => {
                let (a, b) = (builder_enum_type_to_usize(self), builder_enum_type_to_usize(other));
                if a < b {
                    return Ordering::Less;
                }
                Ordering::Greater
            }
        }
    }
}

impl <E: IntoIterator<Item = (K, V)>, A: IntoIterator<Item = R>, R: AsRef<OsStr>, O: AsRef<OsStr>, K: AsRef<OsStr>, V: AsRef<OsStr>, P: AsRef<Path>>
    PartialOrd for Builder<E, P, O, A, K, V, R>
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

type NodeMap = HashMap<String, NodeIndex<u32>>;

fn encode_target_port(t: &Target) -> String {
    vec!["P".to_owned(), control_to_string(&t.control), t.name.clone(), t.port.to_string()].join("#")
}

fn encode_target_control(t: &Target) -> String {
    vec!["C".to_owned(), control_to_string(&t.control), t.name.clone()].join("#")
}

fn decode_string_to_target(s: &str) -> Option<Target> {
    match s.rsplit("#").collect::<Vec<&str>>()[..] {
        [port, name, control, _typ] => {
            Some(Target { control: string_to_control(control), name: name.to_owned(), port: port_str_to_enum(&port) })
        },
        _ => None,
    }
}

type TheGraph = StableGraph::<String, u32, petgraph::Directed, u32>;

struct BuilderSpec<E, P, O, A, K, V, R>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    spec: BTreeSet<Builder<E, P, O, A, K, V, R>>,
    graph: TheGraph,
}

fn get_builder_spec<E, P, O, A, K, V, R>(graph: &TheGraph, nodes: &NodeMap, mut lines: Vec<Line<E, P, O, A, K, V, R>>) -> BTreeSet<Builder<E, P, O, A, K, V, R>>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{


    fn decode_string_to_control(s: &str) -> Option<(Control, String)> {
        match s.split("#").collect::<Vec<&str>>()[..] {
            ["C", t, name] => Some((string_to_control(t), name.to_string())),
            _ => None,
        }
    }

    fn get_command<E, P, O, A, K, V, R>(lines: &mut Vec<Line<E, P, O, A, K, V, R>>, s: &String) -> Option<Builder<E, P, O, A, K, V, R>>
        where E: IntoIterator<Item = (K, V)>,
              A: IntoIterator<Item = R>,
              R: AsRef<OsStr>,
              O: AsRef<OsStr>,
              K: AsRef<OsStr>,
              V: AsRef<OsStr>,
              P: AsRef<Path>,
    {
        match decode_string_to_control(s) {
            Some((Control::Command, name)) => {
                lines.iter()
                    .position(|l| l.name == name)
                    .and_then(|p| Some(lines.remove(p)))
                    .and_then(|l| Some(Builder::Command(Command {
                        name: l.name,
                        spec: l.spec,
                    })))
            },
            Some((Control::Buffer, name)) => Some(Builder::Buffer(Buffer { name })),
            Some((Control::Sink, name)) => Some(Builder::Sink(Sink { name })),
            Some((Control::Tap, name)) => Some(Builder::Tap(Tap { name })),
            None => None,
        }
    }

    fn iterator<E, P, O, A, K, V, R>(graph: &TheGraph, nodes: &NodeMap, mut lines: &mut Vec<Line<E, P, O, A, K, V, R>>, src: NodeIndex, mut r: &mut BTreeSet<Builder<E, P, O, A, K, V, R>>)
        where E: IntoIterator<Item = (K, V)>,
              A: IntoIterator<Item = R>,
              R: AsRef<OsStr>,
              O: AsRef<OsStr>,
              K: AsRef<OsStr>,
              V: AsRef<OsStr>,
              P: AsRef<Path>,
    {

        match get_command(&mut lines, &graph[src]) {
            Some(c) => {
                r.insert(c);
            },
            _ => (),
        }

        for n in graph.neighbors(src) {

            let mut will_continue = true;

            match get_command(&mut lines, &graph[n]) {
                Some(c) => {
                    r.insert(c);
                },
                _ => (),
            }

            match (decode_string_to_target(&graph[src]), decode_string_to_target(&graph[n])) {
                (Some(src_target), Some(dst_target)) => {
                    let join = Builder::Join(Join {
                        src: src_target,
                        dst: dst_target,
                    });
                    // if r.contains(&join) {
                    //     println!("HJ: {:?} {:?}", &graph[src], &graph[n]);
                    //     will_continue = false;
                    // }
                    r.insert(join);
                }
                _ => (),
            }

            if will_continue {
                iterator(&graph, nodes, lines, n, &mut r);
            }

        }
    }

    let mut r: BTreeSet<Builder<E, P, O, A, K, V, R>> = BTreeSet::new();
    for tap in graph.externals(Direction::Incoming) {
        iterator(&graph, &nodes, &mut lines, tap, &mut r);
    }

    r

}

fn increment_id(id: &mut u32) -> u32 {
    *id = *id + 1;
    *id
}

fn increment_port<K>(hm: &mut HashMap<K, usize>, k: K) where K: Eq + std::hash::Hash {
    match hm.get_mut(&k) {
        None => {
            hm.insert(k, 1);
        },
        Some(n) => {
            *n = *n + 1;
        },
    }
}



fn buffer_spec_multi_out(graph: &TheGraph, nodes: &NodeMap, direction: Direction) -> Vec<BufferSpec> {

    fn get_multi_folder(graph: &TheGraph, mut acc: Vec<BufferSpec>, item: &NodeIndex<u32>, direction: Direction) -> Vec<BufferSpec> {

        let mut dst: Vec<Target> = vec![];
        let mut out_count = 0;

        let neighbors = graph.neighbors_directed(*item, direction);

        for neigh in neighbors {
            match decode_string_to_target(&graph[neigh]) {
                Some(t) => {
                    out_count = out_count + 1;
                    dst.push(t)
                },
                None => {},
            }
        }

        if out_count < 2 {
            return acc;
        }

        match decode_string_to_target(&graph[*item]) {
            Some(t) => {
                acc.push(match direction {
                    Direction::Outgoing => BufferSpec { src: vec![t], dst },
                    Direction::Incoming => BufferSpec { dst: vec![t], src: dst },
                });
                acc
            },
            None => acc
        }

    }

    nodes.into_iter().fold(
        vec![],
        |acc, (_node, node_index)| get_multi_folder(&graph, acc, node_index, direction)
    )
}

fn get_port_node(graph: &mut TheGraph, nodes: &mut NodeMap, target: &Target) -> NodeIndex<u32> {
    let s = encode_target_port(target);
    match nodes.get(&s) {
        Some(node) => *node,
        None => {
            let node = graph.add_node(s.clone());
            nodes.insert(s, node);
            node
        },
    }
}

fn get_control_node(graph: &mut TheGraph, nodes: &mut NodeMap, target: &Target) -> NodeIndex<u32> {
    let s = encode_target_control(target);
    match nodes.get(&s) {
        Some(node) => *node,
        None => {
            let node = graph.add_node(s.clone());
            nodes.insert(s, node);
            node
        },
    }
}

fn add_buffers(mut graph: &mut TheGraph, mut nodes: &mut NodeMap, buffer_positions: Vec<BufferSpec>, direction: usize, mut edge_id: &mut u32) {
    for i in 0..buffer_positions.len() {

        let buffer_in = Target {
            control: Control::Buffer,
            name: format!("BUFFER_{}_{}", direction, i),
            port: Port::IN
        };

        let buffer_out = Target {
            control: Control::Buffer,
            name: format!("BUFFER_{}_{}", direction, i),
            port: Port::OUT
        };

        let n_buf = get_control_node(&mut graph, &mut nodes, &buffer_out);
        let n_buf_in = get_port_node(&mut graph, &mut nodes, &buffer_in);
        let n_buf_out = get_port_node(&mut graph, &mut nodes, &buffer_out);

        graph.add_edge(n_buf_in, n_buf, increment_id(&mut edge_id));
        graph.add_edge(n_buf, n_buf_out, increment_id(&mut edge_id));

        for d in &buffer_positions[i].dst {
            for s in &buffer_positions[i].src {
                match (nodes.get(&encode_target_port(s)), nodes.get(&encode_target_port(d))) {
                    (Some(sn), Some(dn)) => {
                        match graph.find_edge(*sn, *dn) {
                            Some(e) => {
                                graph.remove_edge(e);
                                match graph.find_edge(*sn,n_buf_in) {
                                    None => { graph.add_edge(*sn, n_buf_in, increment_id(&mut edge_id)); }
                                    _ => (),
                                }
                                match graph.find_edge(n_buf_out, *dn) {
                                    None => { graph.add_edge(n_buf_out, *dn, increment_id(&mut edge_id)); }
                                    _ => (),
                                }
                            },
                            None => (),
                        }
                    }
                    _ => (),
                }
            }
        }
    }
}

fn add_sinks(mut graph: &mut TheGraph, mut nodes: &mut NodeMap, mut edge_id: &mut u32, desired_sinks: Outputs) {
    for (name, source) in desired_sinks {

        let sink_target = Target { control: Control::Sink, name, port: Port::IN };

        let dst_c = get_control_node(&mut graph, &mut nodes, &sink_target);
        let dst_p = get_port_node(&mut graph, &mut nodes, &sink_target);

        for src in source {
            let src_c = get_control_node(&mut graph, &mut nodes, &src);
            let src_p = get_port_node(&mut graph, &mut nodes, &src);

            for pair in &[(src_c, src_p), (src_p, dst_p), (dst_p, dst_c)] {
                match graph.find_edge(pair.0, pair.1) {
                    None => {
                        graph.add_edge(pair.0, pair.1, increment_id(&mut edge_id));
                    },
                    Some(_) => (),
                }
            }

        }

    }
}



fn identify<E, P, O, A, K, V, R>(lines: Vec<Line<E, P, O, A, K, V, R>>, desired_sinks: Outputs) -> BuilderSpec<E, P, O, A, K, V, R>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{

    let mut edge_id: u32 = 0;
    let mut nodes: NodeMap = HashMap::new();
    let mut graph: TheGraph = StableGraph::new();


    for line in &lines {
        let dst_target = Target { control: Control::Command, name: line.name.clone(), port: Port::IN };
        let dst_c = get_control_node(&mut graph, &mut nodes, &dst_target);
        let dst_p = get_port_node(&mut graph, &mut nodes, &dst_target);

        for src in &line.src {
            let src_c = get_control_node(&mut graph, &mut nodes, &src);
            let src_p = get_port_node(&mut graph, &mut nodes, &src);

            for pair in &[(src_c, src_p), (src_p, dst_p), (dst_p, dst_c)] {
                match graph.find_edge(pair.0, pair.1) {
                    None => {
                        graph.add_edge(pair.0, pair.1, increment_id(&mut edge_id));
                    },
                    Some(_) => (),
                }
            }
        }
    }

    add_sinks(&mut graph, &mut nodes, &mut edge_id, desired_sinks);
    let buffer_spec_in = buffer_spec_multi_out(&graph, &nodes, Direction::Incoming);
    add_buffers(&mut graph, &mut nodes, buffer_spec_in, 0, &mut edge_id);
    let buffer_spec_out = buffer_spec_multi_out(&graph, &nodes, Direction::Outgoing);
    println!("BUFFERS: {:?}", buffer_spec_out);
    add_buffers(&mut graph, &mut nodes, buffer_spec_out, 1, &mut edge_id);

    BuilderSpec {
        spec: get_builder_spec(&graph, &nodes, lines),
        graph,
    }

    // IdentifyNonCommands {
    //     taps: graph.externals(Direction::Incoming).map(|t| string_to_control_name(&graph[t])).collect(),
    //     sinks: graph.externals(Direction::Outgoing).map(|t| string_to_control_name(&graph[t])).collect(),
    //     buffers: vec![],
    // }
}

fn get_test_spec(cmd: &str) -> Specification<HashMap<String, String>, String, String, Vec<String>, String, String, String>
{
    Specification::new(
        HashMap::new() as HashMap<String, String>,
        ".".to_owned(),
        cmd.to_owned(),
        vec!["s/^/ONE: /".to_owned()]
    )
}

#[test]
fn test_identify() {

        // let sed: &OsStr = OsStr::new("sed");
    let lines: Vec<Line<HashMap<String, String>, String, String, Vec<String>, String, String, String>> = vec![
        Line {
            src: vec![Target { control: Control::Tap, name: "TAP".to_owned(), port: Port::OUT }],
            spec: get_test_spec("sed"),
            name: "A".to_owned(),
        },
        Line {
            src: vec![Target { control: Control::Command, name: "A".to_owned(), port: Port::OUT }],
            spec: get_test_spec("grep"),
            name: "B".to_owned(),
        },
    ];

    let mut outputs: Outputs = BTreeMap::new();

    outputs.insert("OUTPUT".to_owned(), vec![
        Target { control: Control::Command, name: "A".to_owned(), port: Port::OUT },
        Target { control: Control::Command, name: "B".to_owned(), port: Port::OUT },
    ]);
    outputs.insert("B_ERRORS".to_owned(), vec![
        Target { control: Control::Command, name: "B".to_owned(), port: Port::ERR }
    ]);
    outputs.insert("A_PURE".to_owned(), vec![
        Target { control: Control::Command, name: "A".to_owned(), port: Port::OUT },
    ]);

    let result = identify(lines, outputs);

    // println!("{:?}", result.spec);
    println!("{}", Dot::new(&result.graph));

    let mut expected: BTreeSet<Builder<HashMap<String, String>, String, String, Vec<String>, String, String, String>> = BTreeSet::new();
    expected.insert(Builder::Join(Join { src: Target { control: Control::Command, name: "A".to_owned(), port: Port::OUT }, dst: Target { control: Control::Buffer, name: "BUFFER_1_0".to_owned(), port: Port::IN } }));
    expected.insert(Builder::Join(Join { src: Target { control: Control::Command, name: "B".to_owned(), port: Port::OUT }, dst: Target { control: Control::Buffer, name: "BUFFER_0_0".to_owned(), port: Port::IN } }));
    expected.insert(Builder::Join(Join { src: Target { control: Control::Command, name: "B".to_owned(), port: Port::ERR }, dst: Target { control: Control::Sink, name: "B_ERRORS".to_owned(), port: Port::IN } }));
    expected.insert(Builder::Join(Join { src: Target { control: Control::Buffer, name: "BUFFER_0_0".to_owned(), port: Port::OUT }, dst: Target { control: Control::Sink, name: "OUTPUT".to_owned(), port: Port::IN } }));
    expected.insert(Builder::Join(Join { src: Target { control: Control::Buffer, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Target { control: Control::Sink, name: "A_PURE".to_owned(), port: Port::IN } }));
    expected.insert(Builder::Join(Join { src: Target { control: Control::Buffer, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Target { control: Control::Command, name: "B".to_owned(), port: Port::IN } }));
    expected.insert(Builder::Join(Join { src: Target { control: Control::Buffer, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Target { control: Control::Buffer, name: "BUFFER_0_0".to_owned(), port: Port::IN } }));
    expected.insert(Builder::Join(Join { src: Target { control: Control::Tap, name: "TAP".to_owned(), port: Port::OUT }, dst: Target { control: Control::Command, name: "A".to_owned(), port: Port::IN } }));

    expected.insert(Builder::Command(Command { name: "A".to_owned(), spec: get_test_spec("sed") }));
    expected.insert(Builder::Command(Command { name: "B".to_owned(), spec: get_test_spec("grep") }));

    expected.insert(Builder::Buffer(Buffer { name: "BUFFER_0_0".to_owned() }));
    expected.insert(Builder::Buffer(Buffer { name: "BUFFER_1_0".to_owned() }));
    expected.insert(Builder::Sink(Sink { name: "A_PURE".to_owned() }));
    expected.insert(Builder::Sink(Sink { name: "B_ERRORS".to_owned() }));
    expected.insert(Builder::Sink(Sink { name: "OUTPUT".to_owned() }));
    expected.insert(Builder::Tap(Tap { name: "TAP".to_owned() }));


    assert_eq!(expected, result.spec);
    // assert_eq!(
    //     IdentifyNonCommands {
    //         taps: vec!["TAP".to_owned()],
    //         sinks: vec!["ERRORS".to_owned(), "OUTPUT".to_owned(), "PURE_A".to_owned()],
    //         buffers: vec![
    //             BufferSpec {
    //                 src: vec![
    //                     Target { control: Control::Command, name: "B".to_owned(), port: Port::OUT },
    //                     Target { control: Control::Command, name: "A".to_owned(), port: Port::OUT },
    //                 ],
    //                 dst: vec![
    //                     Target { control: Control::Sink, name: "OUTPUT".to_owned(), port: Port::IN }
    //                 ]
    //             },
    //             BufferSpec {
    //                 src: vec![
    //                     Target { control: Control::Command, name: "A".to_owned(), port: Port::OUT }
    //                 ],
    //                 dst: vec![
    //                     Target { control: Control::Sink, name: "PURE_A".to_owned(), port: Port::IN },
    //                     Target { control: Control::Sink, name: "OUTPUT".to_owned(), port: Port::IN },
    //                     Target { control: Control::Command, name: "B".to_owned(), port: Port::IN },
    //                 ]
    //             },
    //         ],
    //     },
    //     identify(lines, outputs)
    // );

}



// CONSTRUCT: TAP#T
// CONSTRUCT: BUFFER#BUFFER_1
// CONSTRUCT: BUFFER#BUFFER_2
// CONSTRUCT: COMMAND#A (cmd, args, etc)
// JOIN: TAP#T#OUT BUFFER#BUFFER_1#IN 34


// enum ControlInstance {
//     Buffer(Buffer),
//     Command(Command),
//     Sink(Sink),
//     Tap(Tap),
// }

// type ControlInstanceId = String;

// struct ControlInstanceAndNext {
//     control_instance: ControlInstance,
//     next: Vec<ControlInstanceId>
// }

// type ControlInstanceAndNextStore = HashMap<ControlInstanceId, ControlInstanceAndNext>;




// #[test]
// fn test_identify_multi() {

//     let lines: Vec<Line> = vec![
//         Line {
//             src: vec![Target { name: "TAP".to_owned(), port: Port::OUT }],
//             spec: Specification(),
//             name: "A".to_owned(),
//         },
//         Line {
//             src: vec![Target { name: "TAP".to_owned(), port: Port::OUT }],
//             spec: Specification(),
//             name: "B".to_owned(),
//         },
//         Line {
//             src: vec![
//                 Target { name: "A".to_owned(), port: Port::OUT },
//                 Target { name: "B".to_owned(), port: Port::OUT },
//             ],
//             spec: Specification(),
//             name: "ONE".to_owned(),
//         },
//         Line {
//             src: vec![
//                 Target { name: "A".to_owned(), port: Port::OUT },
//                 Target { name: "B".to_owned(), port: Port::OUT },
//             ],
//             spec: Specification(),
//             name: "TWO".to_owned(),
//         },
//     ];

//     let mut outputs: Outputs = BTreeMap::new();

//     outputs.insert("OUTPUT".to_owned(), vec![
//         Target { name: "ONE".to_owned(), port: Port::OUT },
//         Target { name: "TWO".to_owned(), port: Port::OUT },
//     ]);

//     assert_eq!(
//         IdentifyNonCommands {
//             taps: vec!["TAP".to_owned()],
//             sinks: vec!["OUTPUT".to_owned()],
//             buffers: vec![
//                 BufferSpec { src: vec![Target { name: "TAP".to_owned(), port: Port::OUT }], dst: vec![Target { name: "B".to_owned(), port: Port::IN }, Target { name: "A".to_owned(), port: Port::IN }] },
//                 BufferSpec { src: vec![Target { name: "TWO".to_owned(), port: Port::OUT }, Target { name: "ONE".to_owned(), port: Port::OUT }], dst: vec![Target { name: "OUTPUT".to_owned(), port: Port::IN }] },


//                 BufferSpec {
//                     src: vec![
//                         Target { name: "B".to_owned(), port: Port::OUT },
//                         Target { name: "A".to_owned(), port: Port::OUT }
//                     ],
//                     dst: vec![Target { name: "ONE".to_owned(), port: Port::IN }]
//                 },
//                 BufferSpec {
//                     src: vec![
//                         Target { name: "B".to_owned(), port: Port::OUT },
//                         Target { name: "A".to_owned(), port: Port::OUT }
//                     ],
//                     dst: vec![Target { name: "TWO".to_owned(), port: Port::IN }]
//                 },
//                 BufferSpec {
//                     src: vec![Target { name: "A".to_owned(), port: Port::OUT }],
//                     dst: vec![
//                         Target { name: "TWO".to_owned(), port: Port::IN },
//                         Target { name: "ONE".to_owned(), port: Port::IN }
//                     ]
//                 },
//                 BufferSpec {
//                     src: vec![Target { name: "B".to_owned(), port: Port::OUT }],
//                     dst: vec![
//                         Target { name: "TWO".to_owned(), port: Port::IN },
//                         Target { name: "ONE".to_owned(), port: Port::IN }
//                     ]
//                 }
//             ]

//         },
//         identify(lines, outputs)
//     );

// }
// enum ControlType {
//     Tap,
//     Sink,
//     Buffer,
//     Command
// }

// struct ControlPort {
//     port: String,
//     dest: Vec<Target>
// }

// struct Control {
//     control_type: ControlType,
//     name: String,
//     dest: Vec<ControlPort>
// }

// fn structure(lines: Vec<Line>, identfied: IdentifyNonCommands) -> Vec<Control> {

//     type TheGraph = Graph::<String, u32, petgraph::Directed, u32>;

//     let mut graph: TheGraph = Graph::new();
//     let mut included_control: BTreeSet<String> = BTreeSet::new();
//     let mut included_ports: BTreeSet<Target> = BTreeSet::new();

//     fn add_to_graph(graph: &mut TheGraph, target: &Target, included_control: &mut BTreeSet<String>, included_ports: &mut BTreeSet<Target>) {
//         match included_control.contains(&target.name) {
//             true => (),
//             false => {
//                 graph.add_node(encode_target_control(target));
//             }
//         }
//         match included_ports.contains(&target) {
//             true => (),
//             false => {
//                 graph.add_node(encode_target_port(target));
//             }
//         }
//     }

//     for tap in identfied.taps {
//         add_to_graph(
//             &mut graph,
//             &Target { name: tap, port: Port::OUT },
//             &mut included_control,
//             &mut included_ports,
//         );
//     }

//     for sink in identfied.sinks {
//         add_to_graph(
//             &mut graph,
//             &Target { name: sink, port: Port::IN },
//             &mut included_control,
//             &mut included_ports,
//         );
//     }

//     vec![]
// }

// #[test]
// fn test_structure_tiny() {

//     let lines: Vec<Line> = vec![
//         Line {
//             src: Target { name: "TAP1".to_owned(), port: Port::OUT },
//             spec: Specification(),
//             dst: Target { name: "SINK1".to_owned(), port: Port::IN },
//         },
//     ];

//     let identfied = IdentifyNonCommands {
//         taps: vec!["TAP1".to_owned()],
//         sinks: vec!["SINK1".to_owned()],
//         buffers: vec![],
//     };

//     let expected: Vec<Control> = vec![
//         Control {
//             control_type: ControlType::Tap,
//             name: "TAP1".to_owned(),
//             dest: vec![ControlPort {
//                 port: Port::OUT,
//                 dest: vec![
//                     Target { name: "TAP1-SINK1".to_owned(), port: Port::IN }
//                 ]
//             }]
//         },
//         Control {
//             control_type: ControlType::Tap,
//             name: "TAP1-SINK1".to_owned(),
//             dest: vec![ControlPort {
//                 port: "STDOUT".to_owned(),
//                 dest: vec![
//                     Target { name: "SINK1".to_owned(), port: Port::IN }
//                 ]
//             }]
//         },
//         Control { control_type: ControlType::Sink, name: "SINK1".to_owned(), dest: vec![] },
//     ];

// }

// #[test]
// fn test_structure_big() {

//     let expected: Vec<Control> = vec![
//         Control {
//             control_type: ControlType::Tap,
//             name: "TAP".to_owned(),
//             dest: vec![ControlPort {
//                 port: Port::OUT,
//                 dest: vec![
//                     Target { name: "A".to_owned(), port: Port::IN }
//                 ]
//             }]
//         },
//         Control {
//             control_type: ControlType::Command,
//             name: "A".to_owned(),
//             dest: vec![
//                 ControlPort {
//                     port: "STDOUT".to_owned(),
//                     dest: vec![
//                         Target { name: "BUFFER_1".to_owned(), port: Port::IN }
//                     ]
//                 },
//                 ControlPort {
//                     port: "STDERR".to_owned(),
//                     dest: vec![
//                         Target { name: "STDERR".to_owned(), port: Port::IN }
//                     ]
//                 },
//             ],
//         },
//         Control {
//             control_type: ControlType::Buffer,
//             name: "BUFFER_1".to_owned(),
//             dest: vec![
//                 ControlPort {
//                     port: Port::OUT,
//                     dest: vec![
//                         Target { name: "A_PURE_STDOUT".to_owned(), port: Port::IN },
//                         Target { name: "BUFFER_2".to_owned(), port: Port::IN },
//                         Target { name: "B".to_owned(), port: Port::IN }
//                     ]
//                 }
//             ],
//         },
//         Control {
//             control_type: ControlType::Command,
//             name: "B".to_owned(),
//             dest: vec![
//                 ControlPort {
//                     port: Port::OUT,
//                     dest: vec![
//                         Target { name: "BUFFER_2".to_owned(), port: Port::IN }
//                     ]
//                 }
//             ],
//         },
//         Control {
//             control_type: ControlType::Buffer,
//             name: "BUFFER_2".to_owned(),
//             dest: vec![
//                 ControlPort {
//                     port: Port::OUT,
//                     dest: vec![Target { name: "STDOUT".to_owned(), port: Port::IN }],
//                 }
//             ],
//         },
//         Control { control_type: ControlType::Sink, name: "STDOUT".to_owned(), dest: vec![] },
//         Control { control_type: ControlType::Sink, name: "STDERR".to_owned(), dest: vec![] },
//         Control { control_type: ControlType::Sink, name: "A_PURE_STDERR".to_owned(), dest: vec![] },
//     ];

//     let identfied = IdentifyNonCommands {
//         taps: vec!["TAP".to_owned()],
//         sinks: vec!["STDERR".to_owned(), "STDOUT".to_owned(), "A_PURE_STDOUT".to_owned()],
//         buffers: vec![
//             BufferSpec {
//                 src: vec![
//                     Target { name: "A".to_owned(), port: Port::OUT },
//                     Target { name: "B".to_owned(), port: Port::OUT }
//                 ],
//                 dst: vec![
//                     Target { name: "STDOUT".to_owned(), port: Port::IN }
//                 ]
//             },
//             BufferSpec {
//                 src: vec![
//                     Target { name: "A".to_owned(), port: Port::OUT }
//                 ],
//                 dst: vec![
//                     Target { name: "A_PURE_STDOUT".to_owned(), port: Port::IN },
//                     Target { name: "STDOUT".to_owned(), port: Port::IN },
//                     Target { name: "B".to_owned(), port: Port::IN },
//                 ]
//             },
//         ],
//     };

//     let lines: Vec<Line> = vec![
//         Line {
//             src: Target { name: "TAP".to_owned(), port: Port::OUT },
//             spec: Specification(),
//             dst: Target { name: "A".to_owned(), port: Port::IN },
//         },
//         Line {
//             src: Target { name: "A".to_owned(), port: Port::OUT },
//             spec: Specification(),
//             dst: Target { name: "B".to_owned(), port: Port::IN },
//         },
//         Line {
//             src: Target { name: "A".to_owned(), port: Port::ERR },
//             spec: Specification(),
//             dst: Target { name: "STDERR".to_owned(), port: Port::IN },
//         },
//         Line {
//             src: Target { name: "B".to_owned(), port: Port::OUT },
//             spec: Specification(),
//             dst: Target { name: "STDOUT".to_owned(), port: Port::IN },
//         },
//         Line {
//             src: Target { name: "A".to_owned(), port: Port::OUT },
//             spec: Specification(),
//             dst: Target { name: "STDOUT".to_owned(), port: Port::IN },
//         },
//         Line {
//             src: Target { name: "A".to_owned(), port: Port::OUT },
//             spec: Specification(),
//             dst: Target { name: "A_PURE_STDOUT".to_owned(), port: Port::IN },
//         },
//     ];
// }
//
//
// fn simply_buffers(mut input: Vec<BufferSpec>) -> Vec<BufferSpec> {

//     fn add_dst_to_src(has_full_src: &mut BufferSpec, has_different_dest: BufferSpec) {
//         for d in has_different_dest.dst {
//             if !has_full_src.dst.contains(&d) {
//                 has_full_src.dst.push(d);
//             }
//         }
//     }

//     fn add_src_to_dst(has_full_dst: &mut BufferSpec, has_different_src: BufferSpec) {
//         for s in has_different_src.src {
//             if !has_full_dst.src.contains(&s) {
//                 has_full_dst.src.push(s);
//             }
//         }
//     }

//     fn merge_ar(aa: Vec<Target>, bb: Vec<Target>) -> Vec<Target> {
//         let mut r: Vec<Target> = vec![];
//         for a in aa {
//             if !r.contains(&a) {
//                 r.push(a);
//             }
//         }
//         for b in bb {
//             if !r.contains(&b) {
//                 r.push(b);
//             }
//         }
//         r.sort();
//         r
//     }

//     fn merge_srcs(input: &mut Vec<BufferSpec>, pos_a: usize, pos_b: usize) {
//         let (aa, bb) = match pos_a < pos_b {
//             true => (input.remove(pos_b), input.remove(pos_a)),
//             false => (input.remove(pos_a), input.remove(pos_b))
//         };
//         input.push(BufferSpec {
//             dst: aa.dst,
//             src: merge_ar(aa.src, bb.src),
//         });
//     }

//     fn merge_dsts(input: &mut Vec<BufferSpec>, pos_a: usize, pos_b: usize) {
//         let (aa, bb) = match pos_a < pos_b {
//             true => (input.remove(pos_b), input.remove(pos_a)),
//             false => (input.remove(pos_a), input.remove(pos_b))
//         };
//         input.push(BufferSpec {
//             src: aa.src,
//             dst: merge_ar(aa.dst, bb.dst),
//         });
//     }

//     for i in 0..input.len() {
//         input[i].src.sort();
//         input[i].dst.sort();
//     }

//     let mut keep_going = true;

//     while keep_going {
//         keep_going = false;
//         for i in 0..input.len() {

//             if keep_going {
//                 continue;
//             }

//             // let same_src_pos = input.iter().enumerate().position(|(ii, other)| i != ii && other.src == input[i].src);
//             let same_dst_pos = input.iter().enumerate().position(|(ii, other)| i != ii && other.dst == input[i].dst);

//             // match same_src_pos {
//             //     Some(pos) => {
//             //         println!("MERGE");
//             //         merge_dsts(&mut input, pos, i);
//             //         keep_going = true;
//             //         continue;
//             //     },
//             //     None => (),
//             // }

//             match same_dst_pos {
//                 Some(pos) => {
//                     println!("MERGE");
//                     merge_srcs(&mut input, pos, i);
//                     keep_going = true;
//                 },
//                 None => (),
//             }
//             // match same_src_pos.and_then(|i| { input.get_mut(i) }) {
//             //     None => (),
//             //     Some(mut rss) => {
//             //         add_dst_to_src(&mut rss, input.remove(i));
//             //         continue;
//             //     }
//             // }

//             // match same_dst_pos.and_then(|i| { input.get_mut(i) }) {
//             //     None => {
//             //     },
//             //     Some(mut rsd) => {
//             //         add_src_to_dst(&mut rsd, input.remove(i));
//             //         continue;
//             //     }
//             // }

//             // input.push(input.remove(0));
//         }
//     }

//     input
// }

// #[test]
// fn test_simplify_buffers() {

//     // TODO: WHY!
//     // BufferSpec {
//     //   src: [
//     //     Target { name: "C", port: "OUT" },
//     //     Target { name: "A", port: "OUT" },
//     //     Target { name: "B", port: "OUT" }
//     //   ],
//     //   dst: [Target { name: "ONE", port: "IN" }]
//     // }

//     let mut input = vec![
//         BufferSpec {
//             src: vec![
//                 Target { name: "C".to_owned(), port: Port::OUT },
//                 Target { name: "B".to_owned(), port: Port::OUT },
//                 Target { name: "A".to_owned(), port: Port::OUT }
//             ],
//             dst: vec![Target { name: "ONE".to_owned(), port: Port::IN }]
//         },
//         BufferSpec {
//             src: vec![
//                 Target { name: "B".to_owned(), port: Port::OUT },
//                 Target { name: "A".to_owned(), port: Port::OUT }
//             ],
//             dst: vec![Target { name: "TWO".to_owned(), port: Port::IN }]
//         },
//         BufferSpec {
//             src: vec![Target { name: "A".to_owned(), port: Port::OUT }],
//             dst: vec![
//                 Target { name: "TWO".to_owned(), port: Port::IN },
//                 Target { name: "ONE".to_owned(), port: Port::IN }
//             ]
//         },
//         BufferSpec {
//             src: vec![Target { name: "B".to_owned(), port: Port::OUT }],
//             dst: vec![
//                 Target { name: "TWO".to_owned(), port: Port::IN },
//                 Target { name: "ONE".to_owned(), port: Port::IN }
//             ]
//         }
//     ];

//     let expected = vec![
//         BufferSpec {
//             src: vec![
//                 Target { name: "C".to_owned(), port: Port::OUT },
//             ],
//             dst: vec![Target { name: "ONE".to_owned(), port: Port::IN }]
//         },
//         BufferSpec {
//             src: vec![
//                 Target { name: "A".to_owned(), port: Port::OUT },
//                 Target { name: "B".to_owned(), port: Port::OUT },
//             ],
//             dst: vec![
//                 Target { name: "ONE".to_owned(), port: Port::IN },
//                 Target { name: "TWO".to_owned(), port: Port::IN }
//             ]
//         },
//     ];

//     assert_eq!(expected, simply_buffers(input));

// }


