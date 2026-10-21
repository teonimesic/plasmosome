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
