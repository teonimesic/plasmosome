use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use plasmosome_backend::CellId;

use super::{
    CellHistory, CellJournalFault, CellJournalFaultKind, CellJournalRecord, JournalRuleError,
    parse_record,
};

/// Reads a whole journal, deriving its cell from the parent directory. Missing and empty files
/// return no records. A final-component symlink or non-regular file is an `Io` fault.
///
/// Production recovery must use `read_cell_journal_from` with the file opened directory-relative
/// by core's `CellDir::open_journal`. This convenience opener does not validate ancestors or ACLs.
/// It never writes, repairs or returns a decoded prefix after a fault.
pub fn read_cell_journal(path: &Path) -> Result<Vec<CellJournalRecord>, CellJournalFault> {
    let fault = |detail| CellJournalFault {
        path: path.to_path_buf(),
        line: None,
        lines_parsed: 0,
        kind: CellJournalFaultKind::Io,
        detail,
    };
    let cell = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or_else(|| fault("journal parent has no UTF-8 cell name".to_string()))?;
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(fault(error.to_string())),
    };
    if !file
        .metadata()
        .map_err(|error| fault(error.to_string()))?
        .is_file()
    {
        return Err(fault("journal is not a regular file".to_string()));
    }
    read_cell_journal_from(file, path, &CellId::from(cell))
}

/// Reads every record for `cell` or returns the first fault without a decoded prefix.
/// `path` is diagnostic only. Pass a regular file already opened through the trusted cell
/// directory without following links; this function does not open or validate a path.
pub fn read_cell_journal_from<R: Read>(
    reader: R,
    path: &Path,
    cell: &CellId,
) -> Result<Vec<CellJournalRecord>, CellJournalFault> {
    read_cell_history(reader, path, cell).map(|(records, _)| records)
}

pub(crate) fn read_cell_history<R: Read>(
    reader: R,
    path: &Path,
    cell: &CellId,
) -> Result<(Vec<CellJournalRecord>, CellHistory), CellJournalFault> {
    let mut history = CellHistory::new(cell.clone());
    let records = read_records(reader, path, cell, |record| history.admit(record))?;
    Ok((records, history))
}

pub(crate) fn read_records<R: Read>(
    reader: R,
    path: &Path,
    cell: &CellId,
    mut admit: impl FnMut(&CellJournalRecord) -> Result<(), JournalRuleError>,
) -> Result<Vec<CellJournalRecord>, CellJournalFault> {
    let mut reader = BufReader::new(reader);
    let mut records = Vec::new();
    let mut bytes = Vec::new();
    let mut line = 0;
    loop {
        line += 1;
        bytes.clear();
        let fault = |kind, detail| CellJournalFault {
            path: path.to_path_buf(),
            line: Some(line),
            lines_parsed: records.len() as u64,
            kind,
            detail,
        };
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => return Ok(records),
            Ok(_) => {}
            Err(error) => return Err(fault(CellJournalFaultKind::Io, error.to_string())),
        }
        let Some(record) = bytes.strip_suffix(b"\n") else {
            return Err(fault(
                CellJournalFaultKind::Unparseable,
                "the last record has no final LF".to_string(),
            ));
        };
        let record = parse_record(cell, record)
            .and_then(|record| admit(&record).map(|()| record))
            .map_err(|refusal| fault(refusal.kind, refusal.detail))?;
        records.push(record);
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::cell_journal::{
        AttachmentSource, Change, Decision, JournalEvent, Prepare, Replacement, encode_record,
    };
    use plasmosome_backend::{MockMode, PluginId};

    #[test]
    fn history_reader_reports_each_interrupted_prefix() {
        let prepare = JournalEvent::Prepare(Prepare {
            changes: vec![Change {
                plugin: PluginId::from("p"),
                replacement: Some(Replacement {
                    mock: MockMode::Simulate,
                    source: AttachmentSource::Local,
                    effects: vec![],
                }),
            }],
            force: None,
        });
        for (event, decision) in [
            (JournalEvent::Commit, Decision::Commit),
            (JournalEvent::Abort, Decision::Abort),
        ] {
            let frames = [
                line(1, prepare.clone()),
                line(1, event),
                line(1, JournalEvent::Finish),
            ];
            for count in 1..=3 {
                let bytes = frames[..count].concat();
                let (records, history) =
                    read_cell_history(&bytes[..], Path::new("diagnostic"), &CellId::from("c"))
                        .unwrap();
                assert_eq!(records.len(), count);
                assert_eq!(history.last_generation(), 1);
                match count {
                    1 => assert_eq!(history.open_transaction().unwrap().decision, None),
                    2 => assert_eq!(history.open_transaction().unwrap().decision, Some(decision)),
                    _ => assert!(history.open_transaction().is_none()),
                }
            }
        }
    }

    fn line(generation: u64, event: JournalEvent) -> Vec<u8> {
        encode_record(&CellJournalRecord { generation, event }).unwrap()
    }

    fn good() -> Vec<u8> {
        [line(1, JournalEvent::Abort), line(1, JournalEvent::Finish)].concat()
    }

    fn read(bytes: &[u8]) -> Result<Vec<CellJournalRecord>, CellJournalFault> {
        read_records(
            bytes,
            Path::new("cells/c/ledger.ndjson"),
            &CellId::from("c"),
            |_| Ok(()),
        )
    }

    fn fault_at(bytes: &[u8]) -> (CellJournalFaultKind, Option<u64>, u64) {
        let fault = read(bytes).unwrap_err();
        assert_eq!(fault.path, Path::new("cells/c/ledger.ndjson"));
        (fault.kind, fault.line, fault.lines_parsed)
    }

    struct FailsAfter<'a>(&'a [u8]);

    impl Read for FailsAfter<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Err(io::Error::other("the disk went away"));
            }
            self.0.read(buffer)
        }
    }

    #[test]
    fn empty_input_is_empty_history() {
        assert_eq!(read(b""), Ok(Vec::new()));
        assert_eq!(read(&good()).unwrap().len(), 2);
    }

    #[test]
    fn fault_names_physical_line_and_accepted_prefix() {
        let bytes = [good(), b"{}\n".to_vec()].concat();
        assert_eq!(
            fault_at(&bytes),
            (CellJournalFaultKind::UnsupportedFormat, Some(3), 2)
        );
    }

    #[test]
    fn crlf_counts_one_line() {
        let crlf: Vec<u8> = good()
            .split_inclusive(|byte| *byte == b'\n')
            .flat_map(|framed| [&framed[..framed.len() - 1], b"\r\n"].concat())
            .collect();
        assert_eq!(read(&crlf).unwrap().len(), 2);
        let bytes = [crlf, b"\r\n".to_vec()].concat();
        assert_eq!(
            fault_at(&bytes),
            (CellJournalFaultKind::Unparseable, Some(3), 2)
        );
    }

    #[test]
    fn complete_final_json_without_lf_faults() {
        let mut bytes = [good(), line(2, JournalEvent::Abort)].concat();
        bytes.pop();
        assert_eq!(
            fault_at(&bytes),
            (CellJournalFaultKind::Unparseable, Some(3), 2)
        );
    }

    #[test]
    fn torn_utf8_tail_faults() {
        let bytes = [good(), "{\"é".as_bytes()[..3].to_vec()].concat();
        assert_eq!(
            fault_at(&bytes),
            (CellJournalFaultKind::Unparseable, Some(3), 2)
        );
    }

    #[test]
    fn bad_middle_line_stops_reading() {
        let bytes = [line(1, JournalEvent::Abort), b"\n".to_vec(), good()].concat();
        let mut admitted = 0;
        let fault = read_records(&bytes[..], Path::new("j"), &CellId::from("c"), |_| {
            admitted += 1;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(
            (fault.kind, fault.line, fault.lines_parsed),
            (CellJournalFaultKind::Unparseable, Some(2), 1)
        );
        assert_eq!(admitted, 1);
    }

    #[test]
    fn admit_refusal_becomes_fault_at_its_line() {
        let mut seen = 0;
        let fault = read_records(&good()[..], Path::new("j"), &CellId::from("c"), |_| {
            seen += 1;
            match seen {
                1 => Ok(()),
                _ => Err(JournalRuleError::new(
                    CellJournalFaultKind::EventOrder,
                    "finish twice",
                )),
            }
        })
        .unwrap_err();
        assert_eq!(
            fault,
            CellJournalFault {
                path: "j".into(),
                line: Some(2),
                lines_parsed: 1,
                kind: CellJournalFaultKind::EventOrder,
                detail: "finish twice".to_string(),
            }
        );
        assert_eq!(
            fault.to_string(),
            "cell journal j line 2: EventOrder after 1 accepted records: finish twice"
        );
    }

    #[test]
    fn mid_read_io_error_names_next_line() {
        let first = line(1, JournalEvent::Abort);
        let fault = read_records(
            FailsAfter(&first),
            Path::new("j"),
            &CellId::from("c"),
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(
            (fault.kind, fault.line, fault.lines_parsed),
            (CellJournalFaultKind::Io, Some(2), 1)
        );
        assert!(
            fault.detail.contains("the disk went away"),
            "{}",
            fault.detail
        );
    }
}
