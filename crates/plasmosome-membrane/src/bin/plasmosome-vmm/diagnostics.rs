pub(super) fn line(message: impl std::fmt::Display) -> String {
    let mut line = format!("plasmosome-vmm: {message}").replace(['\n', '\r'], " ");
    let mut end = line.len().min(1023);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line.truncate(end);
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_has_a_utf8_boundary_within_one_kib() {
        for detail in [
            "é".repeat(1024),
            "x".repeat(2048),
            "a\nb\rc".into(),
            String::new(),
        ] {
            let line = super::line(detail);
            assert!(line.len() <= 1024);
            assert!(line.starts_with("plasmosome-vmm: "));
            assert!(line.ends_with('\n'));
            assert_eq!(line.lines().count(), 1);
            assert!(!line.contains('\r'));
        }
    }
}
