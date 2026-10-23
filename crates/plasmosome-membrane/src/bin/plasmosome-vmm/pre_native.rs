use plasmosome_membrane::runtime::vmm_helper::HelperRefusal;

pub(super) fn finish(
    reconfirm: impl FnOnce() -> Result<(), HelperRefusal>,
    loader: impl FnOnce() -> Result<(), HelperRefusal>,
    inventory: impl FnOnce() -> Result<(), HelperRefusal>,
    run: impl FnOnce() -> HelperRefusal,
) -> Result<std::convert::Infallible, HelperRefusal> {
    reconfirm()?;
    loader()?;
    inventory()?;
    Err(run())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plasmosome_membrane::runtime::vmm_helper::HelperExit;
    use std::cell::RefCell;

    #[test]
    fn late_loader_follows_reconfirmation_and_stops_inventory_and_native_dispatch() {
        let stages = ["reconfirm", "loader", "inventory", "run"];
        for failure in 0..stages.len() {
            let calls = RefCell::new(Vec::new());
            let refusal = || HelperRefusal {
                exit: HelperExit::Inventory,
                detail: stages[failure].into(),
            };
            let step = |index| {
                calls.borrow_mut().push(stages[index]);
                if index == failure {
                    Err(refusal())
                } else {
                    Ok(())
                }
            };
            let result = finish(
                || step(0),
                || step(1),
                || step(2),
                || {
                    calls.borrow_mut().push("run");
                    refusal()
                },
            )
            .unwrap_err();
            assert_eq!(result.detail, stages[failure]);
            assert_eq!(*calls.borrow(), stages[..=failure]);
        }
    }
}
