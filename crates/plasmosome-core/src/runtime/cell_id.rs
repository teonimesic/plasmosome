#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn cell_ids_are_cell_dash_one_to_ten_digits_without_a_leading_zero() {
        for (text, number) in [
            ("cell-1", 1),
            ("cell-9", 9),
            ("cell-10", 10),
            ("cell-1000000000", 1_000_000_000),
            ("cell-9999999999", 9_999_999_999),
        ] {
            assert_eq!(parse_cell_id(text), Some(number), "{text}");
        }
        for text in [
            "cell-0",
            "cell-00",
            "cell-01",
            "cell-12345678901",
            "cell-99999999999",
            "cell-",
            "Cell-1",
            "CELL-1",
            "cell1",
            "cell_1",
            "cell--1",
            "cell-+1",
            "cell-1 ",
            " cell-1",
            "cell-1a",
            "cell-1/x",
            "cell-\u{661}",
            "cell-\u{ff11}",
            "cell",
            "",
        ] {
            assert_eq!(parse_cell_id(text), None, "{text:?}");
        }
    }

    #[test]
    fn the_largest_number_is_ten_nines() {
        assert_eq!(MAX_CELL_NUMBER, 9_999_999_999);
        assert_eq!(format_cell_id(MAX_CELL_NUMBER).as_str(), "cell-9999999999");
        assert_eq!(parse_cell_id("cell-10000000000"), None);
    }

    #[test]
    fn a_formatted_id_parses_back_to_its_number() {
        assert_eq!(format_cell_id(7).as_str(), "cell-7");
        for number in [1, 9, 10, MAX_CELL_NUMBER] {
            let id = format_cell_id(number);
            assert_eq!(parse_cell_id(id.as_str()), Some(number), "{id}");
        }
    }

    proptest! {
        #[test]
        fn every_number_in_range_round_trips(number in 1..=MAX_CELL_NUMBER) {
            prop_assert_eq!(parse_cell_id(format_cell_id(number).as_str()), Some(number));
        }

        #[test]
        fn a_number_out_of_range_never_parses(number in MAX_CELL_NUMBER + 1..=u64::MAX) {
            prop_assert_eq!(parse_cell_id(&format!("cell-{number}")), None);
        }
    }
}
