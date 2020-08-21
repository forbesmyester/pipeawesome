use serde::Deserialize;
#[derive(Debug, Deserialize, PartialEq, Clone, Eq, PartialOrd)]
pub enum Port {
    OUT,
    ERR,
    EXIT,
}


pub type ConnectionId = isize;


#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Hash, Copy)]
pub enum SpecType {
    BufferSpec,
    CommandSpec,
    JunctionSpec,
    SinkSpec,
    TapSpec,
}


impl std::fmt::Display for Port {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Port::OUT => write!(f, "O"),
            Port::ERR => write!(f, "E"),
            Port::EXIT => write!(f, "X"),
        }
    }
}

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

pub type ProcessCount = usize;
