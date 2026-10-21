use plasmosome_backend::CellId;
use plasmosome_core::runtime::cell_id::parse_cell_id;
use std::ffi::OsString;

/// Spec 023 §4's argv form, `--config-fd=3 --lock-fd=4 --cell=<id>`, with which the controller
/// starts a cell's `membraned`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellArgs {
    /// The ID after `--cell=`, one that `parse_cell_id` accepts.
    pub cell: CellId,
}

impl CellArgs {
    /// Takes the arguments after `argv[0]`. Returns the form only when there are exactly three, in
    /// this order: `--config-fd=3` and `--lock-fd=4`, spelled exactly so, then `--cell=` and a
    /// cell ID that `parse_cell_id` accepts. Any other list is `None`, including one with an
    /// argument that is not UTF-8.
    pub fn parse(arguments: &[OsString]) -> Option<CellArgs> {
        let [config, lock, cell] = arguments else {
            return None;
        };
        if config != "--config-fd=3" || lock != "--lock-fd=4" {
            return None;
        }
        let id = cell.to_str()?.strip_prefix("--cell=")?;
        parse_cell_id(id)?;
        Some(CellArgs {
            cell: CellId::from(id),
        })
    }
}
