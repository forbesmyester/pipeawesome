#[path = "common_types.rs"]
mod common_types;

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
use serde::Deserialize;
use super::common_types::*;

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord)]
pub enum SpecType {
    CommandSpec,
    BufferSpec,
    SinkSpec,
    TapSpec,
}

fn string_to_control(s: &str) -> SpecType {
    match s {
        "C" => SpecType::CommandSpec,
        "B" => SpecType::BufferSpec,
        "S" => SpecType::SinkSpec,
        "T" => SpecType::TapSpec,
        _ => panic!("Don't know about control {}", s),
    }
}

fn control_to_string(s: &SpecType) -> String {
    match s {
        SpecType::CommandSpec => "C".to_owned(),
        SpecType::BufferSpec => "B".to_owned(),
        SpecType::SinkSpec => "S".to_owned(),
        SpecType::TapSpec => "T".to_owned(),
    }
}

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord)]
pub struct Destination {
    pub spec_type: SpecType,
    pub name: String,
}

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd)]
pub struct Source {
    pub spec_type: SpecType,
    pub name: String,
    pub port: Port,
}

impl Ord for Source {
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

pub type Outputs = BTreeMap<String, Vec<Source>>;

#[derive(Debug)]
pub struct NativeLaunchSpec<E, P, O, A, K, V, R>
    where E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>,
{
    command: O,
    path: Option<P>,
    env: Option<E>,
    args: Option<A>,
}

impl <E: IntoIterator<Item = (K, V)>,
          A: IntoIterator<Item = R>,
          R: AsRef<OsStr>,
          O: AsRef<OsStr>,
          K: AsRef<OsStr>,
          V: AsRef<OsStr>,
          P: AsRef<Path>> NativeLaunchSpec<E, P, O, A, K, V, R> {
    pub fn new(env: E, path: P, command: O, args: A) -> NativeLaunchSpec<E, P, O, A, K, V, R> {
        NativeLaunchSpec { command, path: Some(path), env: Some(env), args: Some(args) }
    }
}


#[derive(Debug, PartialEq)]
pub struct CommandDesire<LS>
{
    src: Vec<Source>,
    spec: LS,
    name: String,
}


#[derive(Debug)]
#[derive(PartialEq)]
struct BufferPosition {
    src: Vec<Source>,
    dst: Vec<Destination>,
}

#[derive(Debug)]
#[derive(PartialEq)]
struct IdentifyNonCommands {
    taps: Vec<String>,
    sinks: Vec<String>,
    buffers: Vec<BufferPosition>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct JoinSpec {
    pub src: Source,
    pub dst: Destination,
}

#[derive(Debug)]
pub struct CommandSpec<L>
{
    pub name: String,
    pub spec: L,
}

impl <L> PartialEq for CommandSpec<L>
{
    fn eq(&self, other: &Self) -> bool {
        self.name.cmp(&other.name) == Ordering::Equal
    }
}

impl <L> Eq for CommandSpec<L> { }

impl <L> Ord for CommandSpec<L> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

impl <L> PartialOrd for CommandSpec<L> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BufferSpec { pub name: String }
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SinkSpec { pub name: String }
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TapSpec { pub name: String }

#[derive(Debug)]
pub enum Builder<L> {
    JoinSpec(JoinSpec),
    CommandSpec(CommandSpec<L>),
    BufferSpec(BufferSpec),
    SinkSpec(SinkSpec),
    TapSpec(TapSpec),
}

fn builder_enum_type_to_usize<L>(b: &Builder<L>) -> usize {
    match b {
        Builder::JoinSpec(_) => 1,
        Builder::CommandSpec(_) => 2,
        Builder::BufferSpec(_) => 3,
        Builder::SinkSpec(_) => 4,
        Builder::TapSpec(_) => 5,
    }
}

impl <L> PartialEq for Builder<L> {
    fn eq(&self, other: &Self) -> bool {
        builder_enum_type_to_usize(self) == builder_enum_type_to_usize(other)
    }
}

impl <L> Eq for Builder<L> { }

impl <L> Ord for Builder<L> {
    fn cmp(&self, other: &Self) -> Ordering {
        let s = builder_enum_type_to_usize(self);
        let o = builder_enum_type_to_usize(other);
        match (self, other) {
            (Builder::JoinSpec(a), Builder::JoinSpec(b)) => { a.cmp(b) },
            (Builder::CommandSpec(a), Builder::CommandSpec(b)) => { a.cmp(b) },
            (Builder::BufferSpec(a), Builder::BufferSpec(b)) => { a.cmp(b) },
            (Builder::SinkSpec(a), Builder::SinkSpec(b)) => { a.cmp(b) },
            (Builder::TapSpec(a), Builder::TapSpec(b)) => { a.cmp(b) },
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

impl <L> PartialOrd for Builder<L> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

type NodeMap = HashMap<String, NodeIndex<u32>>;

fn encode_destination_port(t: &Destination) -> String {
    vec!["P".to_owned(), control_to_string(&t.spec_type), t.name.clone(), "IN".to_owned()].join("#")
}

fn encode_source_port(t: &Source) -> String {
    vec!["P".to_owned(), control_to_string(&t.spec_type), t.name.clone(), t.port.to_string()].join("#")
}

fn encode_destination_control(t: &Destination) -> String {
    vec!["C".to_owned(), control_to_string(&t.spec_type), t.name.clone()].join("#")
}

fn encode_source_control(t: &Source) -> String {
    vec!["C".to_owned(), control_to_string(&t.spec_type), t.name.clone()].join("#")
}


fn source_to_destination(t: Source) -> Destination {
    Destination {
        spec_type: t.spec_type,
        name: t.name,
    }
}


fn decode_string_to_source(s: &str) -> Option<Source> {
    match s.rsplit("#").collect::<Vec<&str>>()[..] {
        [port, name, control, _typ] => {
            Some(Source { spec_type: string_to_control(control), name: name.to_owned(), port: port_str_to_enum(&port) })
        },
        _ => None,
    }
}

type TheGraph = StableGraph::<String, u32, petgraph::Directed, u32>;

#[derive(Debug)]
pub struct Specification<L> {
    pub spec: BTreeSet<Builder<L>>,
    pub graph: TheGraph,
}

fn get_builder_spec<L>(graph: &TheGraph, nodes: &NodeMap, mut lines: Vec<CommandDesire<L>>) -> BTreeSet<Builder<L>> {


    fn decode_string_to_control(s: &str) -> Option<(SpecType, String)> {
        match s.split("#").collect::<Vec<&str>>()[..] {
            ["C", t, name] => Some((string_to_control(t), name.to_string())),
            _ => None,
        }
    }

    fn get_command<L>(lines: &mut Vec<CommandDesire<L>>, s: &String) -> Option<Builder<L>> {
        match decode_string_to_control(s) {
            Some((SpecType::CommandSpec, name)) => {
                lines.iter()
                    .position(|l| l.name == name)
                    .and_then(|p| Some(lines.remove(p)))
                    .and_then(|l| Some(Builder::CommandSpec(CommandSpec {
                        spec: l.spec,
                        name: l.name,
                    })))
            },
            Some((SpecType::BufferSpec, name)) => Some(Builder::BufferSpec(BufferSpec { name })),
            Some((SpecType::SinkSpec, name)) => Some(Builder::SinkSpec(SinkSpec { name })),
            Some((SpecType::TapSpec, name)) => Some(Builder::TapSpec(TapSpec { name })),
            None => None,
        }
    }

    fn iterator<L>(graph: &TheGraph, nodes: &NodeMap, mut lines: &mut Vec<CommandDesire<L>>, src: NodeIndex, mut r: &mut BTreeSet<Builder<L>>) {

        match get_command(&mut lines, &graph[src]) {
            Some(c) => {
                r.insert(c);
            },
            _ => (),
        }

        for n in graph.neighbors(src) {

            let mut tried = false;
            let mut added = false;

            match get_command(&mut lines, &graph[n]) {
                Some(c) => {
                    tried = true;
                    added = r.insert(c) || added;
                },
                _ => (),
            }

            match (decode_string_to_source(&graph[src]), decode_string_to_source(&graph[n]).map(source_to_destination)) {
                (Some(src_target), Some(dst_target)) => {
                    let join = Builder::JoinSpec(JoinSpec {
                        src: src_target,
                        dst: dst_target,
                    });
                    tried = true;
                    added = r.insert(join) || added;
                }
                _ => (),
            }

            if !tried || added {
                iterator(&graph, nodes, lines, n, &mut r);
            }

        }
    }

    let mut r: BTreeSet<Builder<L>> = BTreeSet::new();
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



fn buffer_spec_multi_out(graph: &TheGraph, nodes: &NodeMap, direction: Direction) -> Vec<BufferPosition> {

    fn get_multi_folder(graph: &TheGraph, mut acc: Vec<BufferPosition>, item: &NodeIndex<u32>, direction: Direction) -> Vec<BufferPosition> {

        let mut dst: Vec<Source> = vec![];
        let mut out_count = 0;

        let neighbors = graph.neighbors_directed(*item, direction);

        for neigh in neighbors {
            match decode_string_to_source(&graph[neigh]) {
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

        match decode_string_to_source(&graph[*item]) {
            Some(t) => {
                acc.push(match direction {
                    Direction::Outgoing => BufferPosition { src: vec![t], dst: dst.into_iter().map(source_to_destination).collect() },
                    Direction::Incoming => BufferPosition { src: dst, dst: vec![source_to_destination(t)] },
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

fn get_port_node_from_destination(graph: &mut TheGraph, nodes: &mut NodeMap, target: &Destination) -> NodeIndex<u32> {
    let s = encode_destination_port(target);
    match nodes.get(&s) {
        Some(node) => *node,
        None => {
            let node = graph.add_node(s.clone());
            nodes.insert(s, node);
            node
        },
    }
}

fn get_port_node(graph: &mut TheGraph, nodes: &mut NodeMap, target: &Source) -> NodeIndex<u32> {
    let s = encode_source_port(target);
    match nodes.get(&s) {
        Some(node) => *node,
        None => {
            let node = graph.add_node(s.clone());
            nodes.insert(s, node);
            node
        },
    }
}

fn get_control_node(graph: &mut TheGraph, nodes: &mut NodeMap, target: &Source) -> NodeIndex<u32> {
    let s = encode_source_control(target);
    match nodes.get(&s) {
        Some(node) => *node,
        None => {
            let node = graph.add_node(s.clone());
            nodes.insert(s, node);
            node
        },
    }
}

fn get_control_node_from_destination(graph: &mut TheGraph, nodes: &mut NodeMap, target: &Destination) -> NodeIndex<u32> {
    let s = encode_destination_control(target);
    match nodes.get(&s) {
        Some(node) => *node,
        None => {
            let node = graph.add_node(s.clone());
            nodes.insert(s, node);
            node
        },
    }
}

fn add_buffers(mut graph: &mut TheGraph, mut nodes: &mut NodeMap, buffer_positions: Vec<BufferPosition>, direction: usize, mut edge_id: &mut u32) {
    for i in 0..buffer_positions.len() {

        let buffer_in = Destination {
            spec_type: SpecType::BufferSpec,
            name: format!("BUFFER_{}_{}", direction, i),
        };

        let buffer_out = Source {
            spec_type: SpecType::BufferSpec,
            name: format!("BUFFER_{}_{}", direction, i),
            port: Port::OUT
        };

        let n_buf = get_control_node(&mut graph, &mut nodes, &buffer_out);
        let n_buf_in = get_port_node_from_destination(&mut graph, &mut nodes, &buffer_in);
        let n_buf_out = get_port_node(&mut graph, &mut nodes, &buffer_out);

        graph.add_edge(n_buf_in, n_buf, increment_id(&mut edge_id));
        graph.add_edge(n_buf, n_buf_out, increment_id(&mut edge_id));

        for d in &buffer_positions[i].dst {
            for s in &buffer_positions[i].src {
                match (nodes.get(&encode_source_port(s)), nodes.get(&encode_destination_port(d))) {
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

        let sink_target = Destination { spec_type: SpecType::SinkSpec, name };

        let dst_c = get_control_node_from_destination(&mut graph, &mut nodes, &sink_target);
        let dst_p = get_port_node_from_destination(&mut graph, &mut nodes, &sink_target);

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



pub fn identify<L>(lines: Vec<CommandDesire<L>>, desired_sinks: Outputs) -> Specification<L> {

    let mut edge_id: u32 = 0;
    let mut nodes: NodeMap = HashMap::new();
    let mut graph: TheGraph = StableGraph::new();


    for line in &lines {
        let dst_target = Destination { spec_type: SpecType::CommandSpec, name: line.name.clone() };
        let dst_c = get_control_node_from_destination(&mut graph, &mut nodes, &dst_target);
        let dst_p = get_port_node_from_destination(&mut graph, &mut nodes, &dst_target);

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
    add_buffers(&mut graph, &mut nodes, buffer_spec_out, 1, &mut edge_id);

    Specification {
        // spec: get_builder_spec(&graph, &nodes, lines),
        spec: get_builder_spec(&graph, &nodes, lines),
        graph,
    }

    // IdentifyNonCommands {
    //     taps: graph.externals(Direction::Incoming).map(|t| string_to_control_name(&graph[t])).collect(),
    //     sinks: graph.externals(Direction::Outgoing).map(|t| string_to_control_name(&graph[t])).collect(),
    //     buffers: vec![],
    // }
}

#[derive(Debug, Deserialize, PartialEq, Clone, Eq, PartialOrd)]
pub struct JSONTarget {
    name: String,
    port: Port,
}


impl JSONTarget {
    pub fn to_target(&self, st: SpecType) -> Source {
        Source {
            spec_type: st,
            port: self.port.clone(),
            name: self.name.clone(),
        }
    }
}


#[derive(Debug, Deserialize)]
pub struct JSONLaunchSpec {
    pub command: String,
    pub path: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub args: Option<Vec<String>>,
}


#[derive(Debug, Deserialize)]
pub struct JSONCommandDesire {
    src: Vec<JSONTarget>,
    name: String,
    spec: JSONLaunchSpec,
}

impl JSONCommandDesire {
    pub fn to_command_desire(self, tap_names: &BTreeSet<String>) -> CommandDesire<JSONLaunchSpec> {

        CommandDesire {
            name: self.name,
            src: self.src.iter().map(|jt| {
                jt.to_target(if tap_names.contains(&jt.name) { SpecType::TapSpec } else { SpecType::CommandSpec } )
            }).collect(),
            spec: self.spec
        }
    }
}


#[derive(Debug)]
pub struct Config<LS> {
    pub commands: Vec<CommandDesire<LS>>,
    pub outputs: Outputs,
}


#[derive(Debug, Deserialize)]
pub struct JSONConfig {
    pub outputs: HashMap<String, Vec<JSONTarget>>,
    pub commands: Vec<JSONCommandDesire>,
}

fn find_taps(commands: &Vec<JSONCommandDesire>) -> BTreeSet<String> {
        let mut found_sources: BTreeSet<&str> = BTreeSet::new();
        let mut found_commands: BTreeSet<&str> = BTreeSet::new();
        let mut r: BTreeSet<String> = BTreeSet::new();
        for i in 0..commands.len() {
            found_commands.insert(&commands[i].name);
            for j in 0..commands[i].src.len() {
                found_sources.insert(&commands[i].src[j].name);
            }
        }
        found_sources.difference(&found_commands).map(|&s| s.to_owned()).collect()
}

#[test]
fn test_find_taps() {

    let input: Vec<JSONCommandDesire> = vec![
        JSONCommandDesire {
            name: "INPUT".to_owned(),
            src: vec![
                JSONTarget { name: "FAUCET".to_owned(), port: Port::OUT },
                JSONTarget { name: "ADD_LEADING_ZERO".to_owned(), port: Port::OUT }
            ],
            spec: JSONLaunchSpec {
                command: "cat".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec![])
            },
        },
        JSONCommandDesire {
            name: "GOOD".to_owned(),
            src: vec![JSONTarget { name: "INPUT".to_owned(), port: Port::OUT }],
            spec: JSONLaunchSpec {
                command: "grep".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec!["^........".to_owned()])
            },
        },
        JSONCommandDesire {
            name: "BAD".to_owned(),
            src: vec![JSONTarget { name: "INPUT".to_owned(), port: Port::OUT }],
            spec: JSONLaunchSpec {
                command: "grep".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec!["-v".to_owned(), "^........".to_owned()])
            },
        },
        JSONCommandDesire {
            name: "ADD_LEADING_ZERO".to_owned(),
            src: vec![JSONTarget { name: "BAD".to_owned(), port: Port::OUT }],
            spec: JSONLaunchSpec {
                command: "sed".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec!["s/^/0/".to_owned()])
            },
        }
    ];

    let mut expected: BTreeSet<String> = BTreeSet::new();
    expected.insert("FAUCET".to_owned());
    assert_eq!(expected, find_taps(&input));
}

impl JSONConfig {
    pub fn to_config(mut self) -> Config<JSONLaunchSpec> {

        let tap_names: BTreeSet<String> = find_taps(&self.commands);

        let outputs: BTreeMap<String, Vec<Source>> = self.outputs.drain().map(|(k, jtargets)| {
            (k, jtargets.into_iter().map(|jt| {
                let spec_type = if tap_names.contains(&jt.name) {
                        SpecType::TapSpec
                    } else {
                        SpecType::CommandSpec
                    };
                jt.to_target(spec_type)
            }
            ).collect())
        }).collect();

        let commands: Vec<CommandDesire<JSONLaunchSpec>> = self.commands.into_iter().map(
            |jd| jd.to_command_desire(&tap_names)
        ).collect();

        Config { commands, outputs }

    }

}

#[test]
fn test_identify() {

    fn get_test_spec(cmd: &str) -> NativeLaunchSpec<HashMap<String, String>, String, String, Vec<String>, String, String, String>
    {
        NativeLaunchSpec::new(
            HashMap::new() as HashMap<String, String>,
            ".".to_owned(),
            cmd.to_owned(),
            vec!["s/^/ONE: /".to_owned()]
        )
    }

        // let sed: &OsStr = OsStr::new("sed");
    let lines: Vec<CommandDesire<NativeLaunchSpec<HashMap<String, String>, String, String, Vec<String>, String, String, String>>> = vec![
        CommandDesire {
            src: vec![Source { spec_type: SpecType::TapSpec, name: "TAP".to_owned(), port: Port::OUT }],
            spec: get_test_spec("sed"),
            name: "A".to_owned(),
        },
        CommandDesire {
            src: vec![Source { spec_type: SpecType::CommandSpec, name: "A".to_owned(), port: Port::OUT }],
            spec: get_test_spec("grep"),
            name: "B".to_owned(),
        },
    ];

    let mut outputs: Outputs = BTreeMap::new();

    outputs.insert("OUTPUT".to_owned(), vec![
        Source { spec_type: SpecType::CommandSpec, name: "A".to_owned(), port: Port::OUT },
        Source { spec_type: SpecType::CommandSpec, name: "B".to_owned(), port: Port::OUT },
    ]);
    outputs.insert("B_ERRORS".to_owned(), vec![
        Source { spec_type: SpecType::CommandSpec, name: "B".to_owned(), port: Port::ERR }
    ]);
    outputs.insert("A_PURE".to_owned(), vec![
        Source { spec_type: SpecType::CommandSpec, name: "A".to_owned(), port: Port::OUT },
    ]);

    let result = identify(lines, outputs);

    let mut expected: BTreeSet<Builder<NativeLaunchSpec<HashMap<String, String>, String, String, Vec<String>, String, String, String>>> = BTreeSet::new();
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::CommandSpec, name: "A".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::BufferSpec, name: "BUFFER_1_0".to_owned(), } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::CommandSpec, name: "B".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::BufferSpec, name: "BUFFER_0_0".to_owned(), } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::CommandSpec, name: "B".to_owned(), port: Port::ERR }, dst: Destination { spec_type: SpecType::SinkSpec, name: "B_ERRORS".to_owned(), } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::BufferSpec, name: "BUFFER_0_0".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::SinkSpec, name: "OUTPUT".to_owned(), } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::BufferSpec, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::SinkSpec, name: "A_PURE".to_owned(), } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::BufferSpec, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::CommandSpec, name: "B".to_owned(), } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::BufferSpec, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::BufferSpec, name: "BUFFER_0_0".to_owned(), } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::TapSpec, name: "TAP".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::CommandSpec, name: "A".to_owned(), } }));

    expected.insert(Builder::CommandSpec(CommandSpec { name: "A".to_owned(), spec: get_test_spec("sed") }));
    expected.insert(Builder::CommandSpec(CommandSpec { name: "B".to_owned(), spec: get_test_spec("grep") }));

    expected.insert(Builder::BufferSpec(BufferSpec { name: "BUFFER_0_0".to_owned() }));
    expected.insert(Builder::BufferSpec(BufferSpec { name: "BUFFER_1_0".to_owned() }));
    expected.insert(Builder::SinkSpec(SinkSpec { name: "A_PURE".to_owned() }));
    expected.insert(Builder::SinkSpec(SinkSpec { name: "B_ERRORS".to_owned() }));
    expected.insert(Builder::SinkSpec(SinkSpec { name: "OUTPUT".to_owned() }));
    expected.insert(Builder::TapSpec(TapSpec { name: "TAP".to_owned() }));


    println!("{}", Dot::new(&result.graph));
    assert_eq!(expected, result.spec);

}


#[test]
fn test_identify_loop() {

    let lines: Vec<CommandDesire<JSONLaunchSpec>> = vec![
        CommandDesire {
            name: "INPUT".to_owned(),
            src: vec![
                Source { spec_type: SpecType::TapSpec, name: "FAUCET".to_owned(), port: Port::OUT },
                Source { spec_type: SpecType::CommandSpec, name: "ADD_LEADING_ZERO".to_owned(), port: Port::OUT }
            ],
            spec: JSONLaunchSpec {
                command: "cat".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec![])
            },
        },
        CommandDesire {
            name: "GOOD".to_owned(),
            src: vec![Source { spec_type: SpecType::CommandSpec, name: "INPUT".to_owned(), port: Port::OUT }],
            spec: JSONLaunchSpec {
                command: "grep".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec!["^........".to_owned()])
            },
        },
        CommandDesire {
            name: "BAD".to_owned(),
            src: vec![Source { spec_type: SpecType::CommandSpec, name: "INPUT".to_owned(), port: Port::OUT }],
            spec: JSONLaunchSpec {
                command: "grep".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec!["-v".to_owned(), "^........".to_owned()])
            },
        },
        CommandDesire {
            name: "ADD_LEADING_ZERO".to_owned(),
            src: vec![Source { spec_type: SpecType::CommandSpec, name: "BAD".to_owned(), port: Port::OUT }],
            spec: JSONLaunchSpec {
                command: "sed".to_owned(),
                path: Some(".".to_owned()),
                env: None,
                args: Some(vec!["s/^/0/".to_owned()])
            },
        }
    ];

        // let sed: &OsStr = OsStr::new("sed");
    let mut outputs: Outputs = BTreeMap::new();

    outputs.insert("OUTPUT".to_owned(), vec![
        Source { spec_type: SpecType::CommandSpec, name: "GOOD".to_owned(), port: Port::OUT },
    ]);

    let result = identify(lines, outputs);


    let mut expected: BTreeSet<Builder<JSONLaunchSpec>> = BTreeSet::new();

    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::TapSpec, name: "FAUCET".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::BufferSpec, name: "BUFFER_0_0".to_owned() } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::BufferSpec, name: "BUFFER_0_0".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::CommandSpec, name: "INPUT".to_owned() } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::CommandSpec, name: "INPUT".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::BufferSpec, name: "BUFFER_1_0".to_owned() } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::BufferSpec, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::CommandSpec, name: "GOOD".to_owned() } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::BufferSpec, name: "BUFFER_1_0".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::CommandSpec, name: "BAD".to_owned() } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::CommandSpec, name: "BAD".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::CommandSpec, name: "ADD_LEADING_ZERO".to_owned() } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::CommandSpec, name: "ADD_LEADING_ZERO".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::BufferSpec, name: "BUFFER_0_0".to_owned() } }));
    expected.insert(Builder::JoinSpec(JoinSpec { src: Source { spec_type: SpecType::CommandSpec, name: "GOOD".to_owned(), port: Port::OUT }, dst: Destination { spec_type: SpecType::SinkSpec, name: "OUTPUT".to_owned() } }));

    expected.insert(Builder::CommandSpec(CommandSpec { name: "INPUT".to_owned(), spec: JSONLaunchSpec {
        command: "cat".to_owned(),
        path: Some(".".to_owned()),
        env: None,
        args: Some(vec![])
    } }));
    expected.insert(Builder::CommandSpec(CommandSpec { name: "GOOD".to_owned(), spec: JSONLaunchSpec {
        command: "grep".to_owned(),
        path: Some(".".to_owned()),
        env: None,
        args: Some(vec!["^........".to_owned()])
    } }));
    expected.insert(Builder::CommandSpec(CommandSpec { name: "BAD".to_owned(), spec: JSONLaunchSpec {
        command: "grep".to_owned(),
        path: Some(".".to_owned()),
        env: None,
        args: Some(vec!["-v".to_owned(), "^........".to_owned()])
    } }));
    expected.insert(Builder::CommandSpec(CommandSpec { name: "ADD_LEADING_ZERO".to_owned(), spec: JSONLaunchSpec {
        command: "sed".to_owned(),
        path: Some(".".to_owned()),
        env: None,
        args: Some(vec!["s/^/0/".to_owned()])
    } }));

    expected.insert(Builder::BufferSpec(BufferSpec { name: "BUFFER_0_0".to_owned() }));
    expected.insert(Builder::BufferSpec(BufferSpec { name: "BUFFER_1_0".to_owned() }));
    expected.insert(Builder::TapSpec(TapSpec { name: "FAUCET".to_owned() }));
    expected.insert(Builder::SinkSpec(SinkSpec { name: "OUTPUT".to_owned() }));
    expected.insert(Builder::SinkSpec(SinkSpec { name: "OUTPUT".to_owned() }));

    println!("{}", Dot::new(&result.graph));

    assert_eq!(expected, result.spec);

}
