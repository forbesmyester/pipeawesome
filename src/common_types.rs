use serde::Deserialize;
#[derive(Debug, Deserialize, PartialEq, Clone, Eq, PartialOrd)]
pub enum Port {
    OUT,
    ERR,
}


impl std::fmt::Display for Port {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Port::OUT => write!(f, "O"),
            Port::ERR => write!(f, "E"),
        }
    }
}

pub fn port_str_to_enum(s: &str) -> Port {
    if s == "O" { Port::OUT } else { Port::ERR }
}

