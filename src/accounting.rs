use core::cmp::Ordering;
use std::collections::{HashSet, HashMap};

use csv::Writer;

use crate::config::*;
use crate::failed::*;
use crate::controls::{ConnectionId, Processable, ProcessCount, ProcessStatus, StoppedBy};


#[derive(Debug, PartialEq)]
pub enum AccountingOperation {
    Addition,
    Subtraction,
}

type CsvLine = String;


pub type ControlId = String;
#[derive(Hash, Debug, Clone, PartialEq, Eq)]
pub struct ControlIO(pub SpecType, pub ControlId, pub ConnectionId);
impl ControlIO {
    pub fn to_tuple(&self) -> (SpecType, ControlId, ConnectionId) {
        (self.0, self.1.to_owned(), self.2)
    }
    pub fn to_tuple_ref(&self) -> (&SpecType, &ControlId, &ConnectionId) {
        (&self.0, &self.1, &self.2)
    }
    pub fn as_control(&self) -> Control {
        Control(self.0, self.1.to_owned())
    }
}
#[derive(Hash, Debug, Clone, PartialEq, Eq)]
pub struct Control(pub SpecType, pub ControlId);
impl Control {
    pub fn to_tuple_ref(&self) -> (&SpecType, &ControlId) {
        (&self.0, &self.1)
    }
}


pub type ControlIOIndex = usize;
pub type ControlIndex = usize;

#[derive(Debug, PartialEq)]
pub struct AccountingStatus {
    process_status: ProcessStatus,
    control_index: ControlIndex,
    outbound_ports: Vec<usize>,
    inbound_ports: Vec<usize>,
}

#[derive(Debug)]
pub struct AccountingBuilder {
    control_ios: Vec<ControlIO>,
    controls: Vec<Control>,
    channel_size: usize,
    channel_high_watermark: usize,
    channel_low_watermark: usize,
    sources: HashMap<(SpecType, ControlId), Vec<Option<ControlIO>>>,
    destinations: HashMap<(SpecType, ControlId), Vec<Option<ControlIO>>>,
}


impl<'a> AccountingBuilder {
    pub fn new(channel_size: usize, channel_high_watermark: usize, channel_low_watermark: usize) -> AccountingBuilder {
        AccountingBuilder {
            control_ios: vec![],
            controls: vec![],
            channel_size,
            channel_high_watermark,
            channel_low_watermark,
            sources: HashMap::new(),
            destinations: HashMap::new(),
        }
    }

    pub fn add_join(&mut self, src: ControlIO, dst: ControlIO) {

        let (src_spec_type, src_control_id, src_connection_id) = src.to_tuple();
        let (dst_spec_type, dst_control_id, dst_connection_id) = dst.to_tuple();

        fn resize(v: &mut Vec<Option<ControlIO>>, index: usize, value: ControlIO) {
            while v.len() <= index {
                v.push(None)
            }
            v[index] = Some(value);
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

        let control_src = src.as_control();
        let control_dst = dst.as_control();
        if !self.controls.contains(&control_src) {
            self.controls.push(control_src);
        }
        if !self.controls.contains(&control_dst) {
            self.controls.push(control_dst);
        }

        if !self.control_ios.contains(&src) { self.control_ios.push(src); }
        if !self.control_ios.contains(&dst) { self.control_ios.push(dst); }
    }

    pub fn build(self) -> Option<Accounting> {

        let rev_control_ios = self.control_ios
            .iter()
            .enumerate()
            .fold(HashMap::new(), |mut acc, (i, ci)| {

                let (k1, k2, k3) = ci.to_tuple_ref();

                let mut hm1 = match acc.remove(k1) {
                    None => HashMap::new(),
                    Some(inner1) => inner1,
                };

                let mut hm2 = match hm1.remove(k2) {
                    None => HashMap::new(),
                    Some(inner2) => inner2,
                };

                hm2.insert(*k3, i);
                hm1.insert(k2.to_owned(), hm2);
                acc.insert(*k1, hm1);

                acc

            });

        let rev_controls = self.controls
            .iter()
            .enumerate()
            .fold(HashMap::new(), |mut acc, (i, ci)| {

                let (k1, k2) = ci.to_tuple_ref();

                let mut inner_hm = match acc.remove(k1) {
                    None => HashMap::new(),
                    Some(inner) => inner,
                };
                inner_hm.insert(k2.to_owned(), i);
                acc.insert(*k1, inner_hm);

                acc

            });


        fn extract(
            hm: HashMap<(SpecType, ControlId), Vec<Option<ControlIO>>>,
            rev_control_ios: &HashMap<SpecType, HashMap<ControlId, HashMap<ConnectionId, ControlIOIndex>>>,
            rev_controls: &HashMap<SpecType, HashMap<ControlId, ControlIOIndex>>
            ) -> HashMap<ControlIndex, Vec<ControlIOReference>>
        {

            let mapper_c_ios = |cio: &ControlIO| {
                let (st, ci, p) = cio.to_tuple_ref();
                rev_control_ios.get(st)
                    .and_then(|hm| hm.get(ci))
                    .and_then(|hm2| hm2.get(p))
                    .map(|&u| u)
            };

            let mapper_cs = |cio: &ControlIO| {
                let (st, ci, p) = cio.to_tuple_ref();
                    rev_controls.get(st)
                        .and_then(|hm| hm.get(ci))
                        .map(|&u| (u, *p))
            };

            let mapper = |cio: &ControlIO| {
                match (mapper_c_ios(&cio), mapper_cs(&cio)) {
                    (Some(control_io_index), Some((control_index, connection_id))) => {
                        Some(ControlIOReference {
                            control_io_index,
                            control_index,
                            connection_id
                        })
                    },
                    _ => None,
                }
            };


            let mut r: HashMap<ControlIndex, Vec<ControlIOReference>> = HashMap::new();
            for ((k1, k2), vs) in hm {
                let new_vs: Vec<ControlIOReference> = vs.into_iter().fold(
                    vec![],
                    |mut acc, ov| {
                        match ov.map(|x| mapper(&x)).flatten() {
                            None => acc,
                            Some(v) => {
                                acc.push(v);
                                acc
                            }
                        }
                    }
                );

                let o_k = rev_controls.get(&k1)
                    .and_then(|hm| hm.get(&k2))
                    .map(|&u| u);

                match o_k {
                    Some(k) => { r.insert(k, new_vs); },
                    None => { panic!("Could not find {:?}:{:?} in rev_controls", &k1, &k2); }
                }
            }
            r
        }

        let sources = extract(self.sources, &rev_control_ios, &rev_controls);
        let destinations = extract(self.destinations, &rev_control_ios, &rev_controls);

        let mut pipe_size: HashMap<ControlIndex, Vec<usize>> = HashMap::new();

        for (_k, cirs) in &destinations {
            for cir in cirs {
                Accounting::set_pipe_size(
                    &mut pipe_size,
                    &cir.control_index,
                    &cir.connection_id,
                    0
                );
            }
        }

        Some(Accounting {
            fail_count: 0,
            total_size: 0,
            controls: self.controls,
            control_ios: self.control_ios,
            sources,
            destinations,
            // new_sources: HashMap::new(),
            // new_destinations: HashMap::new(),
            rev_control_ios,
            rev_controls,
            channel_size: self.channel_size,
            channel_high_watermark: self.channel_high_watermark,
            channel_low_watermark: self.channel_low_watermark,
            pipe_size,
            finished: HashSet::new()
        })

    }
}


#[derive(Debug)]
struct ControlIOReference {
    control_io_index: ControlIOIndex,
    control_index: ControlIndex,
    connection_id: ConnectionId,
}

type PipeSizeHash = HashMap<ControlIndex, Vec<usize>>;


#[derive(Debug)]
pub struct Accounting {
    channel_size: usize,
    channel_high_watermark: usize,
    channel_low_watermark: usize,

    control_ios: Vec<ControlIO>,
    controls: Vec<Control>,
    rev_control_ios: HashMap<SpecType, HashMap<ControlId, HashMap<ConnectionId, ControlIOIndex>>>,
    rev_controls: HashMap<SpecType, HashMap<ControlId, ControlIndex>>,
    sources: HashMap<ControlIndex, Vec<ControlIOReference>>,
    destinations: HashMap<ControlIndex, Vec<ControlIOReference>>,

    fail_count: usize,
    total_size: usize,
    pipe_size: PipeSizeHash,
    finished: HashSet<ControlIndex>,
}


impl Accounting {

    fn get_control_index<'b>(&self, st: &SpecType, ci: &ControlId) -> Option<usize> {
        self.rev_controls.get(st).and_then(|hm| hm.get(ci)).map(|&u| u)
    }

    pub fn get_control(&self, i: ControlIndex) -> Option<&Control> {
        self.controls.get(i)
    }

    pub fn convert_processors(&self, by_spec_type_and_control_id: HashMap<(SpecType, ControlId), Box<dyn Processable + Send>>) -> Option<HashMap<ControlIndex, Box<dyn Processable + Send>>> {

        by_spec_type_and_control_id.into_iter().fold(
            Some(HashMap::new()),
            |acc, (k, v)| {
                let (st, ci) = k;
                match (acc, self.get_control_index(&st, &ci)) {
                    (None, _) => None,
                    (_, None) => None,
                    (Some(mut a), Some(i)) => {
                        a.insert(i, v);
                        Some(a)
                    }
                }
            }
        )

    }

    fn update_total_stats(total: &mut usize, entering: bool, count: &usize) {
        match entering {
            true => { *total = *total + count; },
            false => { *total = *total - count; },
        }
    }

    fn get_pipe_sizes<'x>(pipe_size: &'x PipeSizeHash, ci: &ControlIndex) -> Option<&'x Vec<usize>> {
        pipe_size.get(ci)
    }

    fn get_pipe_size(pipe_size: &PipeSizeHash, control_index: &ControlIndex, connection_id: &ConnectionId) -> usize {

        let m = Accounting::get_pipe_sizes(pipe_size, control_index).and_then(|v| v.get(*connection_id as usize));

        match m {
            Some(n) => *n,
            None => 0,
        }

    }

    fn set_pipe_size(mut pipe_size: &mut PipeSizeHash, ci: &ControlIndex, connection_id: &ConnectionId, size: usize) {

        fn ensure_its_there<K: std::hash::Hash + Eq + Clone, V>(hm: &mut HashMap<K, V>, k: K, default_if_not_there: V) {

            match hm.get_mut(&k) {
                Some(_v) => (),
                None => {
                    hm.insert(k.clone(), default_if_not_there);
                }
            };

        }

        // ensure_its_there(&mut pipe_size, ci, HashMap::new());

        // let mut l2 = match pipe_size.get_mut(&ci.0) {
        //     Some(hm) => { hm },
        //     None => panic!("Accounting::set_pipe_size - get_mut after insert failed (1)"),
        // };

        // let mut hm = self.get_control_index(ci.0, ci, 1);
        ensure_its_there(&mut pipe_size, *ci, Vec::new());
        let mut vs = match pipe_size.remove(ci) {
            Some(v) => v,
            None => Vec::with_capacity(size)
        };

        if *connection_id < 0 {
            panic!("Encountered a negative connection id");
        }
        while vs.len() <= *connection_id as usize {
            vs.push(0);
        }
        vs[*connection_id as usize] = size;

        pipe_size.insert(*ci, vs);

    }

    fn update_pipe_size(mut pipe_size: &mut PipeSizeHash, control_index: &ControlIndex, connection_id: &ConnectionId, count: &usize, operation: AccountingOperation) {

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

        let new_size = get_count(
            Accounting::get_pipe_size(&pipe_size, control_index, connection_id)
        ).expect("Negative encountered during update_pipe_size: addition!)");

        Accounting::set_pipe_size(&mut pipe_size, control_index, connection_id, new_size);

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

    fn out_as_csv_line(out: &Vec<String>) -> Result<String, Box<dyn std::error::Error>> {
        let mut wtr = Writer::from_writer(vec![]);
        wtr.write_record(out)?;
        Ok(String::from_utf8(wtr.into_inner()?)?)
    }

    pub fn debug_header(&self) -> String {
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

    pub fn debug_line(&self, control: &ControlIO, count: &usize, op: AccountingOperation) -> String {
        let mut out: Vec<String> = Vec::with_capacity(self.controls.len());
        for c in self.control_ios.iter() {
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

    fn get_of_type(&self, t: SpecType) -> Vec<ControlIndex> {
        let mut r: Vec<ControlIndex> = vec![];
        for index in 0..self.controls.len() {
            match self.controls.get(index) {
                Some(control) => {
                    let (spec_type, _) = control.to_tuple_ref();
                    if *spec_type == t {
                        r.push(index);
                    }
                },
                None => (),
            }
        }
        r
    }

    pub fn get_sinks(&self) -> Vec<ControlIndex> {
        self.get_of_type(SpecType::SinkSpec)
    }

    pub fn get_taps(&self) -> Vec<ControlIndex> {
        self.get_of_type(SpecType::TapSpec)
    }

    pub fn get_finished(&self) -> &HashSet<ControlIndex> {
        &self.finished
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

    pub fn get_accounting_status(&self, control_index: &ControlIndex, process_status: ProcessStatus) -> AccountingStatus {

        let destinations = self.destinations.get(control_index);
        let outbound_ports = match destinations {
            Some(destinations) => {
                let mut sizes: Vec<usize> = Vec::with_capacity(destinations.len());
                for ci in destinations {
                    sizes.push(Accounting::get_pipe_size(&self.pipe_size, &ci.control_index, &ci.connection_id));
                }
                sizes
            },
            None => vec![]
        };

        AccountingStatus {
            process_status,
            control_index: *control_index,
            outbound_ports,
            inbound_ports: match Accounting::get_pipe_sizes(&self.pipe_size, &control_index) {
                Some(v) => v.to_owned(),
                None => vec![],
            }
        }

    }


    pub fn update(&mut self, control_index: &ControlIndex, ps: &ProcessStatus) -> Vec<CsvLine> {

        let mut csv_lines = Vec::with_capacity(ps.wrote_to.len() + ps.read_from.len());

        for (src_connection_id, count) in ps.wrote_to.iter() {
            // let control = ControlIO(spec_type, control_id.to_owned(), *connection_id);
            // csv_lines.push(self.debug_line(&control, count, AccountingOperation::Addition));
            // match self.get_control_io_index(&spec_type, &control_id, connection_id) {
            //     Some(index) => {
            //         Accounting::update_stats(&mut self.leave, &index, count);
            //     },
            //     None => {
            //         panic!("Accounting could not find ControlIOIndex for {:?}:{:?}:{:?}", &spec_type, &control_id, connection_id);
            //     }
            // }
            Accounting::update_total_stats(&mut self.total_size, true, count);

            let o_dst_control_io_ref = match self.destinations.get(control_index) {
                Some(v) => {
                    let src_connection_index = Accounting::outbound_connection_id_to_vec_index(*src_connection_id);
                    v.get(src_connection_index)
                }
                None => None,
            };

            let (dst_control_index, dst_connection_id) = match o_dst_control_io_ref {
                Some(control_io_ref) => (control_io_ref.control_index, control_io_ref.connection_id),
                None => {
                    let (src_spec_type, src_control_id) = self.controls.get(*control_index).unwrap().to_tuple_ref();
                    panic!(
                        "Accounting should have had a destination for {:?}: {:?} but did not",
                        &src_spec_type,
                        &src_control_id
                    );
                }
            };

            Accounting::update_pipe_size(
                &mut self.pipe_size,
                &dst_control_index,
                &dst_connection_id,
                count,
                AccountingOperation::Addition
            );

        }

        for (connection_id, count) in ps.read_from.iter() {
            // let control = ControlIO(spec_type, control_id.to_owned(), *connection_id);
            // csv_lines.push(self.debug_line(&control, count, AccountingOperation::Subtraction));
            // match self.get_control_io_index(&spec_type, &control_id, connection_id) {
            //     Some(index) => {
            //         Accounting::update_stats(&mut self.enter, &index, count);
            //     }
            //     None => {
            //         panic!("Accounting could not find ControlIOIndex for {:?}:{:?}:{:?}", &spec_type, &control_id, connection_id);
            //     }
            // }
            // Accounting::update_stats(&mut self.enter, &control, count);

            Accounting::update_total_stats(&mut self.total_size, false, count);
            Accounting::update_pipe_size(&mut self.pipe_size, &control_index, connection_id, count, AccountingOperation::Subtraction);
        }

        match &ps.stopped_by {
            StoppedBy::ExhaustedInput => {
                self.finished.insert(*control_index);
            }
            _ => (),
        }


        csv_lines

    }

    pub fn update_failed(control_index: &ControlIndex, process_status: &ProcessStatus, failed: &mut Failed) {
        match (process_status.read_from.len() > 0) || (process_status.wrote_to.len() > 0) {
            true => {
                failed.clear();
            },
            false => {
                // println!("FAIL_ADD: {:?}: {:?}: {:?}", &spec_type, &control_id, &process_status);
                failed.insert(*control_index);
            },
        }
    }

    fn get_recommendation_fail(&mut self, failed: &Failed) -> Option<(ProcessCount, ControlIndex)> {

        self.fail_count = self.fail_count + 1;

        let approx_desired_size = self.controls.len() * (
            (self.channel_high_watermark - self.channel_low_watermark / 2) +
            self.channel_low_watermark
        );

        let mut taps = self.get_taps();

        if self.total_size < approx_desired_size {
            let tap_index = taps.remove(self.fail_count % taps.len());
            if !failed.contains(&tap_index) {
                return Some((
                    self.channel_high_watermark - self.channel_low_watermark,
                    tap_index
                ));
            }
        }

        for control_index in (0..self.controls.len()).rev() {
            if !self.finished.contains(&control_index) && !failed.contains(&control_index) {
                return Some((
                    self.channel_high_watermark - self.channel_low_watermark,
                    control_index
                ));
            }
        }

        None
    }

    fn get_recommendation_normal(&self, status: AccountingStatus, failed: &Failed) -> Option<(ProcessCount, ControlIndex)> {

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

        let not_me = |control_index| {
            !failed.contains(&control_index)
        };

        let try_ret_from_vec = |quantity: usize, port: usize, hm: &HashMap<ControlIndex, Vec<ControlIOReference>>| {

            let x = hm.get(&status.control_index)
                .and_then(|v| v.get(port));

            let one = match x {
                Some(dst) => {
                    Some((quantity, dst))
                }
                _ => None
            };

            one.and_then(|rfv| {
                let (quantity, dst) = rfv;
                match not_me(dst.control_index) {
                    true  => Some((quantity, dst.control_index)),
                    false => None,
                }
            })
        };



        let inbound = match Accounting::read_most_from(&status.process_status) {
            None => get_src_index_to_run(&status.inbound_ports, Ordering::Greater),
            Some((port, quant)) => {
                Some((quant, port as usize))
            },
        };
        let outbound = get_src_index_to_run(&status.outbound_ports, Ordering::Greater);

        match (inbound, outbound) {

            // NONE -> HERE(TAP) -->
            (None, Some((out_fill, _out_port)))
                if (out_fill < self.channel_high_watermark) && not_me(status.control_index) => {
                    // println!("CHOICE: 1");
                    Some((self.channel_size - out_fill, status.control_index))
                },
            (None, Some((_, out_port))) => {
                // println!("CHOICE: 2");
                try_ret_from_vec(self.channel_high_watermark - self.channel_low_watermark, out_port, &self.destinations)
            },

            // ??? -> HERE(SINK) -> None
            (Some((in_fill, _in_port)), None) if (in_fill > self.channel_high_watermark) && not_me(status.control_index) => {
                // println!("CHOICE: 3");
                Some((in_fill - self.channel_high_watermark, status.control_index))
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
            (Some((in_fill, in_port)), Some((_out_fill, _out_port))) if in_fill < self.channel_low_watermark => {
                // println!("CHOICE: 5");
                try_ret_from_vec(self.channel_high_watermark - in_fill, in_port, &self.sources)
            }
            (Some((_in_fill, _in_port)), Some((out_fill, out_port))) if out_fill > self.channel_high_watermark => {
                // println!("CHOICE: 7");
                try_ret_from_vec(out_fill - self.channel_high_watermark, out_port, &self.destinations)
            }
            (Some(_), Some((out_fill, _out_port)))
                if (out_fill < self.channel_low_watermark) && not_me(status.control_index) => {
                    // println!("CHOICE: 6");
                    Some((self.channel_high_watermark - out_fill, status.control_index))
                }
            (Some((in_fill, _in_port)), Some(_))
                if (in_fill > self.channel_high_watermark) && not_me(status.control_index) => {
                    // println!("CHOICE: 8");
                    Some((in_fill - self.channel_high_watermark, status.control_index))
                }
            (Some((in_fill, in_port)), Some((out_fill, out_port)))  => {
                let a = try_ret_from_vec(in_fill, in_port, &self.sources);
                let b = try_ret_from_vec(out_fill, out_port, &self.destinations);
                match (a, b) {
                    (None, Some(b)) => Some(b),
                    (Some(a), None) => Some(a),
                    (Some((an, _ai)), Some((bn, bi))) if bn > an => {
                        // println!("CHOICE: 9");
                        Some((bn, bi))
                    },
                    (Some((an, ai)), Some((_bn, _bi))) => {
                        // println!("CHOICE: 10");
                        Some((an, ai))
                    },
                    (None, None) => None,
                }
            }
            _ => {
                panic!("Both a TAP and a SINK?");
            }
        }

    }

    pub fn get_recommendation(&mut self, status: Option<AccountingStatus>, failed: &Failed) -> Option<(ProcessCount, ControlIndex)> {
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
fn test_accounting_get_control_io_index() {
    println!("T1");
    let mut accounting_builder = AccountingBuilder::new(7, 5, 3);
    accounting_builder.add_join(ControlIO(SpecType::TapSpec, "TAP1".to_owned(), -1), ControlIO(SpecType::BufferSpec, "BUF1".to_owned(), 0));
    accounting_builder.add_join(ControlIO(SpecType::BufferSpec, "BUF1".to_owned(), -1), ControlIO(SpecType::CommandSpec, "CMD1".to_owned(), 0));
    accounting_builder.add_join(ControlIO(SpecType::CommandSpec, "CMD1".to_owned(), -1), ControlIO(SpecType::SinkSpec, "SNK1".to_owned(), 0));
    let accounting = accounting_builder.build().unwrap();

    let buffer_control_index = accounting.get_control_index(&SpecType::BufferSpec, &"BUF1".to_owned()).unwrap();
    assert_eq!(
        Some(&Control(SpecType::BufferSpec, "BUF1".to_owned())),
        accounting.controls.get(buffer_control_index)
    );
}

#[test]
fn test_accounting_recommendation() {

    let failed: &Failed = &Failed::new(100);

    let mut accounting_builder = AccountingBuilder::new(7, 5, 3);
    accounting_builder.add_join(ControlIO(SpecType::TapSpec, "TAP1".to_owned(), -1), ControlIO(SpecType::BufferSpec, "BUF1".to_owned(), 0));
    accounting_builder.add_join(ControlIO(SpecType::BufferSpec, "BUF1".to_owned(), -1), ControlIO(SpecType::CommandSpec, "CMD1".to_owned(), 0));
    accounting_builder.add_join(ControlIO(SpecType::CommandSpec, "CMD1".to_owned(), -1), ControlIO(SpecType::SinkSpec, "SNK1".to_owned(), 0));
    let mut accounting = accounting_builder.build().unwrap();

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

    let buffer_control_index = accounting.get_control_index(
            &SpecType::BufferSpec,
            &"BUF1".to_owned(),
        ).unwrap();

    let tap_control_index = accounting.get_control_index(
            &SpecType::TapSpec,
            &"TAP1".to_owned(),
        ).unwrap();

    let cmd_control_index = accounting.get_control_index(
            &SpecType::CommandSpec,
            &"CMD1".to_owned(),
        ).unwrap();


    accounting.update(&tap_control_index, &tap1_ps1);
    accounting.update(&buffer_control_index, &buf1_ps1);
    accounting.update(&cmd_control_index, &cmd1_ps1);

    let expected1 = AccountingStatus {
        process_status: buf1_ps1_expected,
        control_index: buffer_control_index,
        inbound_ports: vec![2],
        outbound_ports: vec![2],
    };

    assert_eq!(expected1, accounting.get_accounting_status(&buffer_control_index, buf1_ps1));
    assert_eq!(
        Some((3, expected1.control_index)),
        accounting.get_recommendation_normal(expected1, failed)
    );

    // =========================================================================

    // TAP1 --(0)--> BUF1(0) --(4)--> CMD1(1) -> SNK1(0)

    let mut buf1_ps2 = ProcessStatus::new();
    buf1_ps2.add_to_read_from(vec![0; 2].into_iter());
    buf1_ps2.add_to_wrote_to(vec![-1; 2].into_iter());
    println!("PS: {:?}", buf1_ps2);
    println!("ACC: {:?}", accounting);
    accounting.update(&buffer_control_index, &buf1_ps2);
    println!("ACC: {:?}", accounting.pipe_size);

    let mut buf1_ps2_expected = ProcessStatus::new();
    buf1_ps2_expected.add_to_read_from(vec![0; 2].into_iter());
    buf1_ps2_expected.add_to_wrote_to(vec![-1; 2].into_iter());

    let expected2 = AccountingStatus {
        process_status: buf1_ps2_expected,
        control_index: accounting.get_control_index(
            &SpecType::BufferSpec,
            &"BUF1".to_owned(),
        ).unwrap(),
        inbound_ports: vec![0],
        outbound_ports: vec![4],
    };

    assert_eq!(expected2, accounting.get_accounting_status(&buffer_control_index, buf1_ps2));
    assert_eq!(
        Some((3, accounting.get_control_index(&SpecType::TapSpec, &"TAP1".to_owned()).unwrap())),
        accounting.get_recommendation_normal(expected2, failed)
    );

}

#[test]
fn test_accounting_buffers() {

    assert_eq!(1, Accounting::outbound_connection_id_to_vec_index(-2));
    assert_eq!(0, Accounting::outbound_connection_id_to_vec_index(-1));

    let mut accounting_builder = AccountingBuilder::new(5, 4, 3);
    accounting_builder.add_join(ControlIO(SpecType::TapSpec, "TAP1".to_owned(), -1), ControlIO(SpecType::BufferSpec, "BUF1".to_owned(), 0));
    accounting_builder.add_join(ControlIO(SpecType::BufferSpec, "BUF1".to_owned(), -1), ControlIO(SpecType::CommandSpec, "CMD1".to_owned(), 0));
    let mut accounting = accounting_builder.build().unwrap();
    println!("ACC: {:?}", accounting);

    let mut tap1_ps = ProcessStatus::new();
    let mut buf1_ps = ProcessStatus::new();
    let mut cmd1_ps = ProcessStatus::new();
    tap1_ps.add_to_wrote_to(vec![-1; 9].into_iter());
    buf1_ps.add_to_read_from(vec![0; 7].into_iter());
    buf1_ps.add_to_wrote_to(vec![-1; 3].into_iter());
    cmd1_ps.add_to_read_from(vec![0].into_iter());

    let buffer_control_index = accounting.get_control_index(
            &SpecType::BufferSpec,
            &"BUF1".to_owned(),
        ).unwrap();

    let tap_control_index = accounting.get_control_index(
            &SpecType::TapSpec,
            &"TAP1".to_owned(),
        ).unwrap();

    let cmd_control_index = accounting.get_control_index(
            &SpecType::CommandSpec,
            &"CMD1".to_owned(),
        ).unwrap();

    accounting.update(&tap_control_index, &tap1_ps);
    accounting.update(&buffer_control_index, &buf1_ps);
    accounting.update(&cmd_control_index, &cmd1_ps);

}



