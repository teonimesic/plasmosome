use super::path::{PathError, RecipePath};
use crate::state::{CELL_SUPERVISOR_SOCKET, CELLS_DIR, CellPathError, validate_cell_id};
use plasmosome_backend::CellId;

const RUNTIME_DIR: &str = "runtime";
const WRITABLE_ROOT: &str = "root.img";
const CONTROL_SOCKET: &str = "control.uds";
const DATA_SOCKET: &str = "data.uds";

/// Spec 023 §3's paths for one cell under an instance root: its supervisor's socket, and the
/// three per-cell paths spec 023 §1 adds to the template to make that cell's recipe. Build them
/// here and nowhere else, so the startup check and `cell.new` measure and complete the same
/// paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellPaths {
    /// `<root>/cells/<cell>/membrane.uds`.
    pub membrane_socket: RecipePath,
    /// `<root>/cells/<cell>/runtime/root.img`, the recipe's `writable_root`.
    pub writable_root: RecipePath,
    /// `<root>/cells/<cell>/runtime/control.uds`, the recipe's `control_path`.
    pub control_path: RecipePath,
    /// `<root>/cells/<cell>/runtime/data.uds`, the recipe's `data_path`.
    pub data_path: RecipePath,
}

/// Why a cell has no paths under an instance root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellPathsError {
    /// The ID cannot name one directory, as [`validate_cell_id`] says.
    Cell(CellPathError),
    /// A path is not canonical. Under a canonical root and with a valid ID, that is only
    /// `TooLong`, for the first of the four paths, in field order, that is too long.
    Path(PathError),
}

impl CellPaths {
    /// Builds `cell`'s paths under `instance_root`, touching no file. Refuses an ID that
    /// [`validate_cell_id`] refuses, then the first path, in field order, that is not canonical.
    pub fn new(instance_root: &RecipePath, cell: &CellId) -> Result<CellPaths, CellPathsError> {
        validate_cell_id(cell).map_err(CellPathsError::Cell)?;
        let directory = format!("{}/{CELLS_DIR}/{}", instance_root.as_str(), cell.as_str());
        let path = |relative: &str| {
            RecipePath::parse(&format!("{directory}/{relative}")).map_err(CellPathsError::Path)
        };
        Ok(CellPaths {
            membrane_socket: path(CELL_SUPERVISOR_SOCKET)?,
            writable_root: path(&format!("{RUNTIME_DIR}/{WRITABLE_ROOT}"))?,
            control_path: path(&format!("{RUNTIME_DIR}/{CONTROL_SOCKET}"))?,
            data_path: path(&format!("{RUNTIME_DIR}/{DATA_SOCKET}"))?,
        })
    }
}

impl std::fmt::Display for CellPathsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CellPathsError::Cell(refusal) => refusal.fmt(f),
            CellPathsError::Path(fault) => fault.fmt(f),
        }
    }
}

impl std::error::Error for CellPathsError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::cell_id::{MAX_CELL_NUMBER, format_cell_id};
    use crate::runtime::path::{PathError, RecipePath};
    use crate::state::validate_cell_id;
    use plasmosome_backend::CellId;

    const MAX_PATH_BYTES: usize = libc::PATH_MAX as usize - 1;

    fn root(text: &str) -> RecipePath {
        RecipePath::parse(text).expect("the test names a canonical root")
    }

    fn root_of(bytes: usize) -> RecipePath {
        root(&format!("/{}", "r".repeat(bytes - 1)))
    }

    #[test]
    fn a_cells_paths_sit_in_its_directory_and_its_runtime_directory() {
        let paths = CellPaths::new(&root("/srv/roots/r7"), &CellId::from("cell-12"))
            .expect("a valid cell under a canonical root has paths");
        assert_eq!(
            paths.membrane_socket.as_str(),
            "/srv/roots/r7/cells/cell-12/membrane.uds"
        );
        assert_eq!(
            paths.writable_root.as_str(),
            "/srv/roots/r7/cells/cell-12/runtime/root.img"
        );
        assert_eq!(
            paths.control_path.as_str(),
            "/srv/roots/r7/cells/cell-12/runtime/control.uds"
        );
        assert_eq!(
            paths.data_path.as_str(),
            "/srv/roots/r7/cells/cell-12/runtime/data.uds"
        );
    }

    #[test]
    fn the_membrane_socket_is_the_one_the_state_module_names() {
        let instance_root = root("/srv/roots/r7");
        let cell = format_cell_id(MAX_CELL_NUMBER);
        let paths = CellPaths::new(&instance_root, &cell).expect("the longest ID has paths");
        assert_eq!(
            paths.membrane_socket.as_path(),
            crate::state::cell_supervisor_socket_path(instance_root.as_path(), &cell)
                .expect("the longest ID is a cell name"),
        );
    }

    #[test]
    fn an_id_that_cannot_name_one_directory_is_refused_as_validate_cell_id_refuses_it() {
        for text in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            let cell = CellId::from(text);
            let expected = validate_cell_id(&cell).expect_err("the ID is not a cell name");
            assert_eq!(
                CellPaths::new(&root("/srv/roots/r7"), &cell),
                Err(CellPathsError::Cell(expected)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_root_too_long_for_the_longest_path_refuses_naming_that_path_length() {
        let cell = CellId::from("cell-1");
        let control_suffix = "/cells/cell-1/runtime/control.uds".len();
        let fits = root_of(MAX_PATH_BYTES - control_suffix);
        assert!(
            CellPaths::new(&fits, &cell).is_ok(),
            "a control path of exactly {MAX_PATH_BYTES} bytes is a path"
        );
        let over = root_of(MAX_PATH_BYTES - control_suffix + 1);
        assert_eq!(
            CellPaths::new(&over, &cell),
            Err(CellPathsError::Path(PathError::TooLong {
                bytes: MAX_PATH_BYTES + 1,
                max: MAX_PATH_BYTES,
            }))
        );
    }

    #[test]
    fn a_refusal_reads_as_its_cause() {
        let cell = CellPathsError::Cell(crate::state::CellPathError::Empty);
        assert_eq!(cell.to_string(), "a cell ID must not be empty");
        let path = CellPathsError::Path(PathError::TooLong { bytes: 9, max: 8 });
        assert_eq!(
            path.to_string(),
            PathError::TooLong { bytes: 9, max: 8 }.to_string()
        );
    }
}
