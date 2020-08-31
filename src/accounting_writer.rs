// #[path = "./config.rs"]
// mod config;
use crate::config::*;

// #[path = "./accounting.rs"]
// mod accounting;
use crate::accounting::*;

// #[path = "./controls.rs"]
// mod controls;
use crate::controls::*;

// #[path = "./stdin_out.rs"]
// mod stdin_out;
use crate::stdin_out::*;

#[derive(Debug)]
pub struct AccountingWriterError(std::io::Error);

impl From<std::io::Error> for AccountingWriterError {
    fn from(sie: std::io::Error) -> AccountingWriterError {
        AccountingWriterError(sie)
    }
}

pub struct AccountingWriterJoinLogItem {
    pub src_spec_type: SpecType,
    pub src_control_id: ControlId,
    pub src_connection_id: ConnectionId,
    pub dst_spec_type: SpecType,
    pub dst_control_id: ControlId,
    pub dst_connection_id: ConnectionId,
}

pub struct AccountingWriter {
    writer: Box<dyn FnMut(&[u8]) -> Result<(), std::io::Error>>,
}

impl AccountingWriter {
    pub fn new(sm: Option<SinkMethod>) -> AccountingWriter {
        AccountingWriter { writer: Box::new(AccountingWriter::get_output(sm)) }
    }

    pub fn write_u8(&mut self, x: &[u8]) -> Result<(), AccountingWriterError> {
        (self.writer)(x).map_err(AccountingWriterError)
    }

    fn get_output(sm: Option<SinkMethod>) -> impl FnMut(&[u8]) -> Result<(), std::io::Error> {
        let mut ow: Option<Box<dyn std::io::Write>> = match sm {
            None => None,
            Some(SinkMethod::STDERR) => {
                let wf = StderrGetWrite {};
                Some(Box::new(wf.get_write()))
            },
            Some(SinkMethod::STDOUT) => {
                let wf = StdoutGetWrite {};
                Some(Box::new(wf.get_write()))
            },
            Some(SinkMethod::FILENAME(filename)) => {
                let wf = FileGetWrite::new(filename);
                Some(Box::new(wf.get_write()))
            }
        };

        move |s: &[u8]| {
            match ow.as_deref_mut() {
                Some(w) => {
                    w.write_all(s)?;
                    w.flush()?;
                    Ok(())
                },
                None => {
                    Ok(())
                }
            }
        }

    }

    pub fn write_join(&mut self, jli: AccountingWriterJoinLogItem) -> Result<(), AccountingWriterError> {
        self.write_u8(format!(
                "ACCOUNTING: JOIN: {:?}#{}##{}#{}##{:?}#{}\n",
                &jli.src_spec_type, &jli.src_control_id, &jli.src_connection_id,
                &jli.dst_connection_id, &jli.dst_spec_type, &jli.dst_control_id
        ).as_bytes())
    }

    pub fn document_control_indices(&mut self, accounting: &Accounting, control_indices: Vec<ControlIndex>) -> Result<(), AccountingWriterError> {
        for control_index in &control_indices {
            let ot = accounting.get_control(*control_index).map(|x| x.to_tuple_ref());
            let s = ot.map(
                |t| format!("ACCOUNTING: CONTROL_INDEX: {}: {:?}#{}\n", control_index, t.0, t.1)
            ).unwrap_or_else(|| format!("ACCOUNTING: CONTROL_INDEX_FAIL: {}\n", control_index));
            self.write_u8(s.as_bytes())?;
        }
        Ok(())
    }

    pub fn document_process_status(&mut self, control_index: ControlIndex, process_status: &ProcessStatus) -> Result<(), AccountingWriterError> {
        match serde_json::to_string(&process_status) {
            Err(_e) => {
                panic!("Wierdly we could not JSON Serialize {:?}", &process_status);
            }
            Ok(json) => {
                let s = format!(
                    "ACCOUNTING: PROCESS_STATUS: {}, {}\n",
                    control_index,
                    &json
                );
                self.write_u8(s.as_bytes())
            }
        }
    }

}

