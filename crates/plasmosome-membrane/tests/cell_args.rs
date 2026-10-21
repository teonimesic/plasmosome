use plasmosome_backend::CellId;
use plasmosome_membrane::cell_args::CellArgs;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

const FORM: [&str; 3] = ["--config-fd=3", "--lock-fd=4", "--cell=cell-1"];

fn arguments(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

#[test]
fn the_three_arguments_in_order_with_a_cell_id_give_the_cell() {
    for cell in ["cell-1", "cell-9999999999"] {
        let third = format!("--cell={cell}");
        let parsed = CellArgs::parse(&arguments(&["--config-fd=3", "--lock-fd=4", &third]));
        assert_eq!(
            parsed,
            Some(CellArgs {
                cell: CellId::from(cell)
            }),
            "{cell}"
        );
    }
}

#[test]
fn the_three_arguments_in_any_other_order_are_not_the_form() {
    let [config, lock, cell] = FORM;
    for order in [
        [config, cell, lock],
        [lock, config, cell],
        [lock, cell, config],
        [cell, config, lock],
        [cell, lock, config],
    ] {
        assert_eq!(CellArgs::parse(&arguments(&order)), None, "{order:?}");
    }
}

#[test]
fn a_missing_or_extra_argument_is_not_the_form() {
    let [config, lock, cell] = FORM;
    let lists: [&[&str]; 7] = [
        &[],
        &[config],
        &[config, lock],
        &[config, lock, cell, "extra"],
        &["extra", config, lock, cell],
        &[config, lock, cell, cell],
        &[config, config, lock, cell],
    ];
    for list in lists {
        assert_eq!(CellArgs::parse(&arguments(list)), None, "{list:?}");
    }
}

#[test]
fn a_cell_that_is_not_a_cell_id_is_not_the_form() {
    for cell in [
        "--cell=../x",
        "--cell=",
        "--cell=cell-0",
        "--cell=cell-01",
        "--cell=cell-10000000000",
        "--cell=Cell-1",
        "--cell=cell-1 ",
        "--cell=cell-1/x",
        "--cell",
        "--cell cell-1",
        "--cellcell-1",
        "--cell==cell-1",
        "--Cell=cell-1",
        "cell-1",
    ] {
        let list = ["--config-fd=3", "--lock-fd=4", cell];
        assert_eq!(CellArgs::parse(&arguments(&list)), None, "{cell:?}");
    }
}

#[test]
fn the_descriptor_arguments_are_spelled_exactly() {
    for (config, lock) in [
        ("--config-fd=03", "--lock-fd=4"),
        ("--config-fd=3 ", "--lock-fd=4"),
        (" --config-fd=3", "--lock-fd=4"),
        ("--config-fd=5", "--lock-fd=4"),
        ("--config-fd", "--lock-fd=4"),
        ("--config-fd=3", "--lock-fd=4=4"),
        ("--config-fd=3", "--lock-fd=04"),
        ("--config-fd=3", "--lock-fd=5"),
        ("--config-fd=3", "--lock-fd"),
        ("--config-fd=4", "--lock-fd=3"),
        ("--CONFIG-FD=3", "--lock-fd=4"),
    ] {
        let list = [config, lock, "--cell=cell-1"];
        assert_eq!(CellArgs::parse(&arguments(&list)), None, "{list:?}");
    }
}

#[test]
fn an_argument_that_is_not_utf8_is_not_the_form() {
    for position in 0..FORM.len() {
        let mut list = arguments(&FORM);
        let mut bytes = list[position].clone().into_vec();
        bytes.push(0xff);
        list[position] = OsString::from_vec(bytes);
        assert_eq!(CellArgs::parse(&list), None, "argument {position}");
    }
}
