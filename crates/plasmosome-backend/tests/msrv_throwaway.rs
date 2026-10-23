use std::cell::Cell;

fn shorten<'short, 'long: 'short>(cell: Cell<&'long mut i32>) -> Cell<&'short mut dyn Send> {
    cell
}

#[test]
fn a_mut_reference_unsize_coercion_may_shorten_its_lifetime() {
    let mut value = 1_i32;
    let _shorter = shorten(Cell::new(&mut value));
}
