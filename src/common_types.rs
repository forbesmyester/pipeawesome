use serde::Deserialize;
#[derive(Debug, Deserialize, PartialEq, Clone, Eq, PartialOrd)]
pub enum Port {
    OUT,
    ERR,
    EXIT,
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

