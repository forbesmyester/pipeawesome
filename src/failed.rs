use std::time::{Duration, Instant };
use std::collections::HashSet;
use std::convert::TryFrom;

use crate::common_types::ControlIndex;

#[derive(Debug)]
pub struct Failed (Option<Instant>, HashSet<ControlIndex>, u128);

impl Failed {

    pub fn till_clear(&self) -> Duration {
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

    pub fn clear(&mut self) {
        self.0 = None;
        self.1.clear()
    }

    pub fn contains(&self, x: &ControlIndex) -> bool {
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

    pub fn insert(&mut self, k: ControlIndex) -> bool {
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

    pub fn new(fail_clear_millis: u128) -> Failed {
        Failed (None, HashSet::new(), fail_clear_millis)
    }
}


