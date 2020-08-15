mod controls;
mod config;

#[path = "common_types.rs"]
mod common_types;

use std::collections::HashSet;
use petgraph::stable_graph::{ StableGraph, NodeIndex };
use petgraph::{ Direction };
use petgraph::dot::Dot;
use petgraph::algo::astar;
use clap::{Arg as ClapArg, App as ClapApp};

use csv::Writer;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::convert::TryFrom;
use std::path::Path;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, SyncSender, Receiver};
use std::cmp::{ Ord, Ordering };

use self::controls::*;
use self::config::*;
use self::common_types::Port;

const CHANNEL_SIZE: usize = 16;
const CHANNEL_LOW_WATERMARK: usize = 4;
const CHANNEL_HIGH_WATERMARK: usize = 8;

type ControlId = String;
#[derive(Hash, Debug, Clone)]
pub struct ControlInput(pub SpecType, pub ControlId, pub ConnectionId);
impl ControlInput {
    fn to_tuple(&self) -> (SpecType, ControlId, ConnectionId) {
        (self.0, self.1.to_owned(), self.2)
    }
}
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

#[derive(PartialEq, Debug)]
enum AccountingOperation {
    Addition,
    Subtraction,
}

type CsvLine = String;

#[derive(Debug)]
struct Failed (Option<Instant>, HashSet<(SpecType, ControlId)>, u128);

impl Failed {
    fn till_clear(&self) -> Duration {
        match self.0 {
            Some(inst) => {
                if Instant::now().duration_since(inst).as_millis() > self.2 {
                    return Duration::from_millis(0)
                }
                let clear_inst = match u64::try_from(self.2).ok() {
                    Some(n) => Instant::now() + Duration::from_millis(n),
                    None => panic!("The clear time for fail log is too long (>u64)"),
                };
                clear_inst.duration_since(inst)
            }
            None => Duration::from_millis(0)
        }
    }
    fn clear(&mut self) {
        self.0 = None;
        self.1.clear()
    }
    fn contains(&self, x: &(SpecType, ControlId)) -> bool {
        match self.0 {
            Some(inst) => {
                if Instant::now().duration_since(inst).as_millis() > self.2 {
                    return false;
                }
                self.1.contains(x)
            }
            None => false,
        }
    }
    fn insert(&mut self, k: (SpecType, ControlId)) -> bool {
        match self.0 {
            Some(inst) => {
                if Instant::now().duration_since(inst).as_millis() > self.2 {
                    self.0 = Some(Instant::now());
                    self.1.clear();
                }
            }
            None => {
                self.0 = Some(Instant::now());
            },
        }
        self.1.insert(k)
    }
    fn new(fail_clear_millis: u128) -> Failed {
        Failed (None, HashSet::new(), fail_clear_millis)
    }
}

#[derive(Debug, PartialEq)]
struct AccountingStatus {
    process_status: ProcessStatus,
    spec_type: SpecType,
    control_id: ControlId,
    outbound_ports: Vec<usize>,
    inbound_ports: Vec<usize>,
}

#[derive(Debug)]
struct AccountingBuilder {
    controls: Vec<ControlInput>,
    channel_size: usize,
    channel_high_watermark: usize,
    channel_low_watermark: usize,
    sources: HashMap<(SpecType, ControlId), Vec<Option<ControlInput>>>,
    destinations: HashMap<(SpecType, ControlId), Vec<Option<ControlInput>>>,
    pipe_size: PipeSizeHash,
    enter: HashMap<ControlInput, usize>,
    leave: HashMap<ControlInput, usize>,
    finished: BTreeSet<ControlId>,
}


impl AccountingBuilder {
    fn new(channel_size: usize, channel_high_watermark: usize, channel_low_watermark: usize) -> AccountingBuilder {
        AccountingBuilder {
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

    fn add_join(&mut self, src: ControlInput, dst: ControlInput) {

        println!("J: {:?} {:?}", src, dst);
        Accounting::set_pipe_size(&mut self.pipe_size, &dst, 0);
        let (src_spec_type, src_control_id, src_connection_id) = src.to_tuple();
        let (dst_spec_type, dst_control_id, dst_connection_id) = dst.to_tuple();

        fn resize(v: &mut Vec<Option<ControlInput>>, index: usize, value: ControlInput) {
            while v.len() <= index {
                v.push(None)
            }
            println!("V: {:?} [{}] = {:?}", v, index, value);
            v[index] = Some(value);
            println!("Vv: {:?} [{}]", v, index);
        }

        let index = Accounting::outbound_connection_id_to_vec_index(src_connection_id);
        match self.destinations.get_mut(&(src_spec_type, src_control_id.clone())) {
            Some(mut v) => {
                resize(&mut v, index, dst.clone());
            }
            None => {
                let mut v = vec![];
                resize(&mut v, index, dst.clone());
                self.destinations.insert((src_spec_type, src_control_id), v);
            }
        }

        match self.sources.get_mut(&(dst_spec_type, dst_control_id.clone())) {
            Some(mut v) => {
                resize(&mut v, dst_connection_id as usize, src.clone());
            }
            None => {
                let mut v = vec![];
                resize(&mut v, dst_connection_id as usize, src.clone());
                self.sources.insert((dst_spec_type, dst_control_id), v);
            }
        }

        if !self.controls.contains(&src) { self.controls.push(src); }
        if !self.controls.contains(&dst) { self.controls.push(dst); }
    }

    fn build(self) -> Accounting {

        fn extract(hm: HashMap<(SpecType, ControlId), Vec<Option<ControlInput>>>) -> HashMap<SpecType, HashMap<ControlId, Vec<ControlInput>>> {
            let mut r: HashMap<SpecType, HashMap<ControlId, Vec<ControlInput>>> = HashMap::new();
            for ((k1, k2), vs) in hm {
                let new_vs: Vec<ControlInput> = vs.into_iter().fold(
                    vec![],
                    |mut acc, ov| {
                        match ov {
                            None => acc,
                            Some(v) => {
                                acc.push(v);
                                acc
                            }
                        }
                    }
                );
                let mut inner_hm = match r.remove(&k1) {
                    None => HashMap::new(),
                    Some(inner) => inner,
                };
                inner_hm.insert(k2, new_vs);
                r.insert(k1, inner_hm);
            }
            r
        }

        Accounting {
            fail_count: 0,
            total_size: 0,
            controls: self.controls,
            channel_size: self.channel_size,
            channel_high_watermark: self.channel_high_watermark,
            channel_low_watermark: self.channel_low_watermark,
            sources: extract(self.sources),
            destinations: extract(self.destinations),
            pipe_size: self.pipe_size,
            enter: self.enter,
            leave: self.leave,
            finished: self.finished,
        }
    }
}

#[derive(Debug)]
struct Accounting {
    fail_count: usize,
    total_size: usize,
    controls: Vec<ControlInput>,
    channel_size: usize,
    channel_high_watermark: usize,
    channel_low_watermark: usize,
    sources: HashMap<SpecType, HashMap<ControlId, Vec<ControlInput>>>,
    destinations: HashMap<SpecType, HashMap<ControlId, Vec<ControlInput>>>,
    pipe_size: PipeSizeHash,
    enter: HashMap<ControlInput, usize>,
    leave: HashMap<ControlInput, usize>,
    finished: BTreeSet<ControlId>,
}

type Quantity = usize;
type BufferQuantity = usize;
type FailureCount = usize;

enum QueueElement {
    Finished(ControlInput),
    Source(ControlInput, Instant),
    Stopped(ControlInput, Quantity, BufferQuantity, FailureCount, Instant),
    Available(ControlInput, Quantity, BufferQuantity, Instant),
}


struct AmountInChannel(usize);

impl Accounting {

    fn update_total_stats(total: &mut usize, entering: bool, count: &usize) {
        match entering {
            true => { *total = *total + count; },
            false => { *total = *total - count; },
        }
    }

    fn update_stats(e_or_l: &mut HashMap<ControlInput, usize>, control_input: &ControlInput, count: &usize) {
        match e_or_l.get_mut(control_input) {
            None => { e_or_l.insert(control_input.to_owned(), *count); },
            Some(current) => { *current = *current + count },
        }
    }

    fn get_pipe_sizes<'a>(pipe_size: &'a PipeSizeHash, st: &SpecType, ci: &ControlId) -> Option<&'a Vec<usize>> {

        pipe_size.get(st).and_then(|hm| hm.get(ci))

    }

    fn get_pipe_size(pipe_size: &PipeSizeHash, ci: &ControlInput) -> usize {

        // let c2: usize = ((-1 as isize) - ci.2) as usize;
        let m = Accounting::get_pipe_sizes(pipe_size, &ci.0, &ci.1).and_then(|v| v.get(ci.2 as usize));

        match m {
            Some(n) => *n,
            None => 0,
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
                while v.len() <= ci.2 as usize {
                    v.push(0);
                }
                v[ci.2 as usize] = size;
            }
            None => panic!("Accounting::set_pipe_size - get_mut after insert failed (2)"),
        }

    }

    fn update_pipe_size<'a>(mut pipe_size: &mut PipeSizeHash, control_input: &'a ControlInput, count: &usize, operation: AccountingOperation) -> Result<usize, &'a ControlInput> {

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

        let new_size = get_count(Accounting::get_pipe_size(&pipe_size, control_input));
        match new_size {
            None => {
                Err(control_input)
            },
            Some(current_size) => {
                Accounting::set_pipe_size(&mut pipe_size, control_input, current_size);
                Ok(current_size)
            },
        }

    }

    fn outbound_connection_id_to_vec_index(outbound_connection_id: ConnectionId) -> usize {
        if outbound_connection_id > -1 {
            panic!(
                "outbound_connection_id_to_vec_index: You passed a positive number ({}), this must not be an outbound connection_id!",
                outbound_connection_id
            );
        }
        (0 - (outbound_connection_id + 1)) as usize
    }

    fn get_destination(&self, src_spec_type: &SpecType, src_control_id: &ControlId, src_connection_id: &ConnectionId) -> Option<ControlInput> {
        let index = Accounting::outbound_connection_id_to_vec_index(*src_connection_id);
        let x = match self.destinations.get(&src_spec_type).and_then(|h| h.get(src_control_id)) {
            Some(v) => v.get(index),
            None => None,
        };
        match x {
            None => None,
            Some(ci) => Some(ci.to_owned())
        }
    }

    fn out_as_csv_line(out: &Vec<String>) -> Result<String, Box<dyn std::error::Error>> {
        let mut wtr = Writer::from_writer(vec![]);
        wtr.write_record(out)?;
        Ok(String::from_utf8(wtr.into_inner()?)?)
    }

    fn debug_header(&self) -> String {
        let mut out: Vec<String> = Vec::with_capacity(self.controls.len());
        for c in self.controls.iter() {
            out.push(format!("{:?}", c));
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

    fn get_taps(&self) -> Vec<(SpecType, ControlId)> {
        let mut r: Vec<(SpecType, ControlId)> = vec![];
        for control in &self.controls {
            let (spec_type, control_id, _port) = control.to_tuple();
            if spec_type == SpecType::TapSpec {
                r.push((spec_type, control_id));
            }
        }
        r
        // match self.pipe_size.get(&SpecType::TapSpec) {
        //     None => vec![],
        //     Some(hm) => {
        //         hm.iter()
        //             .map(|(k, _v)| (SpecType::TapSpec, k.to_owned()))
        //             .collect()
        //     },
        // }
    }

    fn read_most_from(pr: &ProcessStatus) -> Option<(ConnectionId, usize)> {
        pr.read_from.iter()
            .fold(None, |acc, (conn_id, quant)| {
                match acc {
                    None => Some((*conn_id, *quant)),
                    Some((_, acc_quant)) if quant > &acc_quant => {
                        Some((*conn_id, *quant))
                    }
                    _ => acc
                }
            })
    }

    fn get_accounting_status(&self, spec_type: &SpecType, control_id: &ControlId, process_status: ProcessStatus) -> AccountingStatus {


        let destinations = self.destinations.get(spec_type).and_then(|h| h.get(control_id));
        let outbound_ports = match destinations {
            Some(destinations) => {
                let mut sizes: Vec<usize> = Vec::with_capacity(destinations.len());
                for ci in destinations {
                    sizes.push(Accounting::get_pipe_size(&self.pipe_size, ci));
                }
                sizes
            },
            None => vec![]
        };


        AccountingStatus {
            process_status,
            spec_type: *spec_type,
            control_id: control_id.to_owned(),
            outbound_ports,
            inbound_ports: match Accounting::get_pipe_sizes(&self.pipe_size, spec_type, control_id) {
                Some(v) => v.to_owned(),
                None => vec![],
            }
        }

    }



    fn update(&mut self, spec_type: SpecType, control_id: ControlId, ps: &ProcessStatus) -> Vec<CsvLine> {

        let mut csv_lines = Vec::with_capacity(ps.wrote_to.len() + ps.read_from.len());

        for (connection_id, count) in ps.wrote_to.iter() {
            let control = ControlInput(spec_type, control_id.to_owned(), *connection_id);
            csv_lines.push(self.debug_line(&control, count, AccountingOperation::Addition));
            Accounting::update_stats(&mut self.leave, &control, count);
            Accounting::update_total_stats(&mut self.total_size, true, count);
            match self.get_destination(&spec_type, &control_id, connection_id) {
                Some(dst) => {
                    let new_size = Accounting::update_pipe_size(
                        &mut self.pipe_size,
                        &dst,
                        count,
                        AccountingOperation::Addition
                    ).expect("Negative encountered during update_pipe_size: addition!)");
                },
                None => {
                    panic!("Accounting should have had a destination for {:?}, but did not\n\n{:?}", &control, self.destinations);
                }
            };
        }
        for (connection_id, count) in ps.read_from.iter() {
            let control = ControlInput(spec_type, control_id.to_owned(), *connection_id);
            csv_lines.push(self.debug_line(&control, count, AccountingOperation::Subtraction));
            Accounting::update_stats(&mut self.enter, &control, count);
            Accounting::update_total_stats(&mut self.total_size, false, count);
            match Accounting::update_pipe_size(&mut self.pipe_size, &control, count, AccountingOperation::Subtraction) {
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
                self.finished.insert(control_id.to_owned());
            }
            _ => (),
        }


        csv_lines

    }

    fn get_nexts(&self, spec_type: SpecType, control_id: ControlId) -> Vec<(SpecType, ControlId)> {

        match spec_type {
            SpecType::CommandSpec => vec![],
            SpecType::SinkSpec => vec![],
            _ => {
                self.destinations.get(&spec_type).and_then(|hm| hm.get(&control_id))
                    .and_then(|vs| {
                        Some(vs.iter()
                            .map(|v| {
                                (v.0, v.1.to_owned())
                            })
                            .collect())
                    })
                    .unwrap_or(vec![])
            }
        }

    }

    fn update_failed(spec_type: &SpecType, control_id: &ControlId, process_status: &ProcessStatus, failed: &mut Failed) {
        match (process_status.read_from.len() > 0) || (process_status.wrote_to.len() > 0) {
            true => {
                failed.clear();
            },
            false => {
                // println!("FAIL_ADD: {:?}: {:?}: {:?}", &spec_type, &control_id, &process_status);
                failed.insert((*spec_type, control_id.to_owned()));
            },
        }
    }

    fn list_controls(&self) -> Vec<(SpecType, ControlId)> {
        let mut r: Vec<(SpecType, ControlId)> = self.get_taps();
        for (spec_type, hm) in &self.sources {
            for (control_id, _v) in hm {
                r.push((*spec_type, control_id.to_owned()))
            }
        }
        r
    }

    fn get_recommendation_fail(&mut self, failed: &Failed) -> Option<(ProcessCount, SpecType, ControlId)> {

        self.fail_count = self.fail_count + 1;

        let approx_desired_size = (self.leave.len() + 1) * (
            (self.channel_high_watermark - self.channel_low_watermark / 2) +
            self.channel_low_watermark
        );

        let mut taps = self.get_taps();

        if self.total_size < approx_desired_size {
            let tap = taps.remove(self.fail_count % taps.len());
            if !failed.contains(&(tap.0, tap.1.clone())) {
                return Some((
                    self.channel_high_watermark - self.channel_low_watermark,
                    tap.0,
                    tap.1
                ));
            }
        }

        for control in self.list_controls().iter().rev() {
            if !failed.contains(control) {
                return Some((
                    self.channel_high_watermark - self.channel_low_watermark,
                    control.0,
                    control.1.to_owned()
                ));
            }
        }

        None
    }

    fn get_recommendation_normal(&self, status: AccountingStatus, failed: &Failed) -> Option<(ProcessCount, SpecType, ControlId)> {

        let get_src_index_to_run = |vs: &Vec<usize>, desired: Ordering| {
            let mut r: Option<(usize, usize)> = None;
            for i in 0..vs.len() {
                match r {
                    None => {
                        r = Some((vs[i], i));
                    },
                    Some((other_hunger, _other_i)) => {
                        if vs[i].cmp(&other_hunger) == desired {
                            r = Some((vs[i], i));
                        }
                    },
                }
            }

            r
        };

        let ret_from_vec = |quantity: usize, port: usize, hm: &HashMap<SpecType, HashMap<ControlId, Vec<ControlInput>>>| {

            let hm = hm.get(&status.spec_type).and_then(|hm| hm.get(&status.control_id))
                .and_then(|v| v.get(port));

            match hm {
                Some(dst) => {
                    let three = dst.to_tuple();
                    Some((quantity, three.0, three.1))
                }
                _ => None
            }
        };

        let inbound = match Accounting::read_most_from(&status.process_status) {
            None => get_src_index_to_run(&status.inbound_ports, Ordering::Greater),
            Some((port, quant)) => {
                Some((quant, port as usize))
            },
        };
        let outbound = get_src_index_to_run(&status.outbound_ports, Ordering::Greater);

        let not_me = |st: &SpecType, ci: &ControlId| {
            !failed.contains(&(*st, ci.to_owned()))
        };

        let try_ret_from_vec = |quantity: usize, port: usize, hm: &HashMap<SpecType, HashMap<ControlId, Vec<ControlInput>>>| {
            ret_from_vec(quantity, port, hm).and_then(|rfv| {
                let (quantity, spec_type, control_id) = rfv;
                match not_me(&spec_type, &control_id) {
                    true => Some((quantity, spec_type, control_id)),
                    false => None,
                }
            })
        };

        match (inbound, outbound) {

            // NONE -> HERE(TAP) -->
            (None, Some((out_fill, out_port)))
                if (out_fill < self.channel_high_watermark) && not_me(&status.spec_type, &status.control_id) => {
                    // println!("CHOICE: 1");
                    Some((self.channel_size - out_fill, status.spec_type, status.control_id))
                },
            (None, Some((_, out_port))) => {
                // println!("CHOICE: 2");
                try_ret_from_vec(self.channel_high_watermark - self.channel_low_watermark, out_port, &self.destinations)
            },

            // ??? -> HERE(SINK) -> None
            (Some((in_fill, in_port)), None) if (in_fill > self.channel_high_watermark) && not_me(&status.spec_type, &status.control_id) => {
                // println!("CHOICE: 3");
                Some((in_fill - self.channel_high_watermark, status.spec_type, status.control_id))
            }
            // (Some((in_fill, in_port)), None) if (in_fill > 0) && not_me(&status.spec_type, &status.control_id) => {
            //     println!("CHOICE: 3");
            //     Some((in_fill, status.spec_type, status.control_id))
            // }
            (Some((in_fill, in_port)), None) if in_fill < self.channel_low_watermark  => {
                // println!("CHOICE: 4");
                try_ret_from_vec(self.channel_high_watermark - in_fill, in_port, &self.sources)
            }
            (Some((in_fill, in_port)), None) => {
                // println!("CHOICE: 4.5");
                try_ret_from_vec(in_fill, in_port, &self.sources)
            }

            // ??? -> HERE -> ???
            (Some((in_fill, in_port)), Some((out_fill, out_port))) if in_fill < self.channel_low_watermark => {
                // println!("CHOICE: 5");
                try_ret_from_vec(self.channel_high_watermark - in_fill, in_port, &self.sources)
            }
            (Some((in_fill, in_port)), Some((out_fill, out_port))) if out_fill > self.channel_high_watermark => {
                // println!("CHOICE: 7");
                try_ret_from_vec(out_fill - self.channel_high_watermark, out_port, &self.destinations)
            }
            (Some((in_fill, in_port)), Some((out_fill, out_port)))
                if (out_fill < self.channel_low_watermark) && not_me(&status.spec_type, &status.control_id) => {
                    // println!("CHOICE: 6");
                    Some((self.channel_high_watermark - out_fill, status.spec_type, status.control_id))
                }
            (Some((in_fill, in_port)), Some((out_fill, out_port)))
                if (in_fill > self.channel_high_watermark) && not_me(&status.spec_type, &status.control_id) => {
                    // println!("CHOICE: 8");
                    Some((in_fill - self.channel_high_watermark, status.spec_type, status.control_id))
                }
            (Some((in_fill, in_port)), Some((out_fill, out_port)))  => {
                let a = try_ret_from_vec(in_fill, in_port, &self.sources);
                let b = try_ret_from_vec(out_fill, out_port, &self.destinations);
                match (a, b) {
                    (None, Some(b)) => Some(b),
                    (Some(a), None) => Some(a),
                    (Some((an, ast, aci)), Some((bn, bst, bci))) if bci > aci => {
                        // println!("CHOICE: 9");
                        Some((bn, bst, bci))
                    },
                    (Some((an, ast, aci)), Some((bn, bst, bci))) => {
                        // println!("CHOICE: 10");
                        Some((an, ast, aci))
                    },
                    (None, None) => None,
                }
            }
            _ => {
                panic!("Both a TAP and a SINK?");
            }
        }

    }

    fn get_recommendation(&mut self, status: Option<AccountingStatus>, failed: &Failed) -> Option<(ProcessCount, SpecType, ControlId)> {
        match status {
            Some(s) => match self.get_recommendation_normal(s, &failed) {
                None => self.get_recommendation_fail(&failed),
                Some(s) => Some(s),
            },
            None => self.get_recommendation_fail(&failed),
        }
    }
}

#[test]
fn test_accounting_recommendation() {

    let failed: &Failed = &Failed::new(100);

    let mut accounting_builder = AccountingBuilder::new(7, 5, 3);
    accounting_builder.add_join(ControlInput(SpecType::TapSpec, "TAP1".to_owned(), -1), ControlInput(SpecType::BufferSpec, "BUF1".to_owned(), 0));
    accounting_builder.add_join(ControlInput(SpecType::BufferSpec, "BUF1".to_owned(), -1), ControlInput(SpecType::CommandSpec, "CMD1".to_owned(), 0));
    accounting_builder.add_join(ControlInput(SpecType::CommandSpec, "CMD1".to_owned(), -1), ControlInput(SpecType::SinkSpec, "SNK1".to_owned(), 0));
    let mut accounting = accounting_builder.build();

    // TAP1 --(2)--> BUF1(0) --(2)--> CMD1(1) -> SNK1(0)

    let mut tap1_ps1 = ProcessStatus::new();
    let mut buf1_ps1 = ProcessStatus::new();
    let mut cmd1_ps1 = ProcessStatus::new();
    tap1_ps1.add_to_wrote_to(vec![-1; 5].into_iter());
    buf1_ps1.add_to_read_from(vec![0; 3].into_iter());
    buf1_ps1.add_to_wrote_to(vec![-1; 3].into_iter());
    cmd1_ps1.add_to_read_from(vec![0].into_iter());

    let mut buf1_ps1_expected = ProcessStatus::new();
    buf1_ps1_expected.add_to_read_from(vec![0; 3].into_iter());
    buf1_ps1_expected.add_to_wrote_to(vec![-1; 3].into_iter());


    accounting.update(SpecType::TapSpec, "TAP1".to_owned(), &tap1_ps1);
    accounting.update(SpecType::BufferSpec, "BUF1".to_owned(), &buf1_ps1);
    accounting.update(SpecType::CommandSpec, "CMD1".to_owned(), &cmd1_ps1);

    let expected1 = AccountingStatus {
        process_status: buf1_ps1_expected,
        spec_type: SpecType::BufferSpec,
        control_id: "BUF1".to_owned(),
        inbound_ports: vec![2],
        outbound_ports: vec![2],
    };

    assert_eq!(expected1, accounting.get_accounting_status(&SpecType::BufferSpec, &"BUF1".to_owned(), buf1_ps1));
    assert_eq!(
        Some((3, SpecType::BufferSpec, "BUF1".to_owned())),
        accounting.get_recommendation_normal(expected1, failed)
    );

    // =========================================================================

    // TAP1 --(0)--> BUF1(0) --(4)--> CMD1(1) -> SNK1(0)

    let mut buf1_ps2 = ProcessStatus::new();
    buf1_ps2.add_to_read_from(vec![0; 2].into_iter());
    buf1_ps2.add_to_wrote_to(vec![-1; 2].into_iter());
    println!("PS: {:?}", buf1_ps2);
    println!("ACC: {:?}", accounting);
    accounting.update(SpecType::BufferSpec, "BUF1".to_owned(), &buf1_ps2);
    println!("ACC: {:?}", accounting.pipe_size);

    let mut buf1_ps2_expected = ProcessStatus::new();
    buf1_ps2_expected.add_to_read_from(vec![0; 2].into_iter());
    buf1_ps2_expected.add_to_wrote_to(vec![-1; 2].into_iter());

    let expected2 = AccountingStatus {
        process_status: buf1_ps2_expected,
        spec_type: SpecType::BufferSpec,
        control_id: "BUF1".to_owned(),
        inbound_ports: vec![0],
        outbound_ports: vec![4],
    };

    assert_eq!(expected2, accounting.get_accounting_status(&SpecType::BufferSpec, &"BUF1".to_owned(), buf1_ps2));
    assert_eq!(
        Some((3, SpecType::TapSpec, "TAP1".to_owned())),
        accounting.get_recommendation_normal(expected2, failed)
    );

}

#[test]
fn test_accounting_buffers() {

    assert_eq!(1, Accounting::outbound_connection_id_to_vec_index(-2));
    assert_eq!(0, Accounting::outbound_connection_id_to_vec_index(-1));

    let mut accounting_builder = AccountingBuilder::new(5, 4, 3);
    accounting_builder.add_join(ControlInput(SpecType::TapSpec, "TAP1".to_owned(), -1), ControlInput(SpecType::BufferSpec, "BUF1".to_owned(), 0));
    accounting_builder.add_join(ControlInput(SpecType::BufferSpec, "BUF1".to_owned(), -1), ControlInput(SpecType::CommandSpec, "CMD1".to_owned(), 0));
    let mut accounting = accounting_builder.build();
    println!("ACC: {:?}", accounting);

    let mut tap1_ps = ProcessStatus::new();
    let mut buf1_ps = ProcessStatus::new();
    let mut cmd1_ps = ProcessStatus::new();
    tap1_ps.add_to_wrote_to(vec![-1; 9].into_iter());
    buf1_ps.add_to_read_from(vec![0; 7].into_iter());
    buf1_ps.add_to_wrote_to(vec![-1; 3].into_iter());
    cmd1_ps.add_to_read_from(vec![0].into_iter());


    accounting.update(SpecType::TapSpec, "TAP1".to_owned(), &tap1_ps);
    accounting.update(SpecType::BufferSpec, "BUF1".to_owned(), &buf1_ps);
    accounting.update(SpecType::CommandSpec, "CMD1".to_owned(), &cmd1_ps);

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
    //   * Max       = 10
    //   * Stuffed   = 7 - 9 (When any sources are)
    //   * Satisfied = 3 - 6
    //   * Hungry    = 1 - 3 (When all sources are beneath)
    //   * Starved   = 0
    //
    // If any control is Full turn off (relevant) taps.
    // If more than a quarter of controls are Stuffed turn of (relevant) taps.
    // If all OK turn on (relevant) taps.
    // If more than a third Hungry turn on (relevant) taps.
    //
    // If Hungry, run Source till either Source = Hungry or Self = Stuffed
    // If Stuffed and Destination less so, run Self until Destination <= Self
    //
    // Hunger level defined by
    //
    // If any above low water mark Satisfied
    // If all empty then Starved
    // If all above high water mark Stuffed
    // If all full then Max
    //
    // TODO: Desire for:
    //         * Below low mark to run till hits high water mark (or full for buffers)
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

    let mut accounting_builder = AccountingBuilder::new(16, 8, 4);

    for b in builders {
        let r = match b {
            Builder::JoinSpec(j) => {
                match controls.join(&j) {
                    Some((c1, c2)) => {
                        accounting_builder.add_join(
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

    let mut accounting = accounting_builder.build();
    let mut processors = controls.get_processors();


    let jjoin_handle = std::thread::spawn(move || {

        let mut loop_number: usize = 0;
        let debug: bool = true;

        println!("TAPS: {:?}", accounting.get_taps());
        print!("CSV: {}", accounting.debug_header());

        let mut currents: Vec<Option<(ProcessCount, (SpecType, ControlId), ControlId)>> = accounting.get_taps()
            .into_iter()
            .map(|t| Some((CHANNEL_HIGH_WATERMARK, (t.0, t.1.clone()), t.1)))
            .collect();

        // let get_src_for_index = |spec_type: SpecType, control_id: ControlId, i: usize| {
        //     let o = accounting.sources
        //         .get(&(spec_type, control_id))
        //         .and_then(|&v| v.get(i));
        // };

        let mut failed: Failed = Failed::new(100);
        let mut inst: Instant = Instant::now();
        loop {
            loop_number = loop_number + 1;

            // println!("CURRENTS: {:?}", currents);
            // println!("PIPE:     {:?}", &accounting.pipe_size);
            for current in &mut currents {
                match current {
                    Some((count, spec_control, tap_id)) => {

                        let (spec_type, control_id) = spec_control;

                        let next_status = match processors.get_mut(&(*spec_type, control_id.clone())) {
                            None => None,
                            Some(processor) => {
                                let process_status = processor.process(*count);
                                let csv_lines = accounting.update(
                                    *spec_type,
                                    control_id.to_owned(),
                                    &process_status
                                );
                                // for line in csv_lines {
                                //     print!("CSV: {}", line);
                                // }
                                Accounting::update_failed(spec_type, control_id, &process_status, &mut failed);
                                let next_status = accounting.get_accounting_status(&spec_type, &control_id, process_status);
                                Some(next_status)
                            }
                        };


                        // println!("=====================");
                        // println!("PS1: {:?}", &accounting.pipe_size);
                        // println!("CUR: {:?}: {:?}", &spec_type, &control_id);
                        // println!("STA: {:?}", &next_status);


                        let rec = accounting.get_recommendation(next_status, &mut failed);

                        match rec {
                            Some((read_count, next_spec_type, next_control_id)) => {
                                *count = read_count;
                                *spec_control = (next_spec_type, next_control_id);
                                // *current = Some((read_count, (next_spec_type, next_control_id), tap_id));
                            }
                            None => {
                                std::thread::sleep(failed.till_clear());
                            }
                        }


                    },
                    None => (),
                }
            };



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

        }
    });


    jjoin_handle.join().unwrap();

}

