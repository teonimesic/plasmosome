#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::backend::{BackendError, Capability, GrantKind, LedgerEntry};
    use crate::fixtures::{self, broker, mount, proxy_map, session_file, uds_socket};
    use crate::universe::{CellId, CellOwner, GrantId, OsState, PluginId, UniverseOp};

    fn owner(plugin: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from("cell-1"),
            plugin: PluginId::from(plugin),
        }
    }

    fn op(capability: Capability) -> UniverseOp {
        fixtures::op(GrantId::new(), owner("github-pr"), capability)
    }

    fn at(address: &UniverseOp, owner: CellOwner, capability: Capability) -> UniverseOp {
        fixtures::op(address.id(), owner, capability)
    }

    fn issued(op: &UniverseOp) -> LedgerEntry {
        LedgerEntry {
            handle: op.handle(),
            owner: op.owner().clone(),
            capability: op.capability(),
            kind: GrantKind::Hot,
        }
    }

    fn incomplete(op: &UniverseOp) -> IncompleteEffect {
        IncompleteEffect {
            operation: op.clone(),
        }
    }

    fn standing(ops: &[&UniverseOp]) -> OsState {
        let mut state = OsState::new();
        for op in ops {
            state.insert(op.object()).unwrap();
        }
        state
    }

    fn conflict(op: &UniverseOp) -> BackendError {
        BackendError::IdentityConflict {
            class: op.class().as_str(),
            id: op.id(),
        }
    }

    fn no_holding(op: &UniverseOp) -> BackendError {
        BackendError::Fault(format!(
            "issued record {} has no standing or incomplete holding",
            op.handle()
        ))
    }

    #[test]
    fn a_complete_account_keeps_every_row_of_its_three_collections() {
        let granted = op(mount("/code", "/workspace"));
        let applied = op(proxy_map("api.github.com", "splice"));
        let unfinished = op(broker("egressd"));
        let state = standing(&[&granted, &applied]);
        let snapshot = EnforcementSnapshot::new(
            state.clone(),
            vec![issued(&granted)],
            vec![incomplete(&unfinished)],
        )
        .unwrap();
        assert_eq!(snapshot.state(), &state);
        assert_eq!(snapshot.grants(), [issued(&granted)]);
        assert_eq!(snapshot.incomplete(), [incomplete(&unfinished)]);
        assert!(!snapshot.is_empty());
    }

    #[test]
    fn an_issued_record_whose_holding_is_incomplete_is_accepted() {
        let failed_withdrawal = op(uds_socket("/run/ak/egressd.uds"));
        let snapshot = EnforcementSnapshot::new(
            OsState::new(),
            vec![issued(&failed_withdrawal)],
            vec![incomplete(&failed_withdrawal)],
        )
        .unwrap();
        assert_eq!(snapshot.grants(), [issued(&failed_withdrawal)]);
        assert_eq!(snapshot.incomplete(), [incomplete(&failed_withdrawal)]);
        assert!(snapshot.state().is_empty());
    }

    #[test]
    fn an_incomplete_effect_is_at_its_operations_address() {
        let unfinished = op(session_file("/skills/pr.md"));
        assert_eq!(incomplete(&unfinished).handle(), unfinished.handle());
    }

    #[test]
    fn a_repeated_incomplete_address_is_refused_even_with_an_identical_operation() {
        let first = op(session_file("/skills/pr.md"));
        let another_owner = at(&first, owner("audit"), session_file("/skills/pr.md"));
        for repeated in [first.clone(), another_owner] {
            assert_eq!(
                EnforcementSnapshot::new(
                    OsState::new(),
                    Vec::new(),
                    vec![incomplete(&first), incomplete(&repeated)],
                ),
                Err(conflict(&first)),
                "{repeated:?}"
            );
        }
    }

    #[test]
    fn an_address_both_standing_and_incomplete_is_refused() {
        let held = op(mount("/code", "/workspace"));
        let another_source = at(&held, owner("github-pr"), mount("/other", "/workspace"));
        for unfinished in [held.clone(), another_source] {
            assert_eq!(
                EnforcementSnapshot::new(
                    standing(&[&held]),
                    Vec::new(),
                    vec![incomplete(&unfinished)],
                ),
                Err(conflict(&held)),
                "{unfinished:?}"
            );
        }
    }

    #[test]
    fn a_repeated_issued_address_is_refused_even_with_an_identical_record() {
        let granted = op(proxy_map("api.github.com", "splice"));
        let other_kind = LedgerEntry {
            kind: GrantKind::GenerationBound,
            ..issued(&granted)
        };
        for repeated in [issued(&granted), other_kind] {
            assert_eq!(
                EnforcementSnapshot::new(
                    standing(&[&granted]),
                    vec![issued(&granted), repeated.clone()],
                    Vec::new(),
                ),
                Err(conflict(&granted)),
                "{repeated:?}"
            );
        }
    }

    #[test]
    fn an_issued_record_that_disagrees_with_its_row_in_owner_or_capability_is_refused() {
        let granted = op(mount("/code", "/workspace"));
        let records = [
            issued(&at(&granted, owner("audit"), mount("/code", "/workspace"))),
            issued(&at(
                &granted,
                CellOwner {
                    cell: CellId::from("cell-2"),
                    plugin: PluginId::from("github-pr"),
                },
                mount("/code", "/workspace"),
            )),
            issued(&at(
                &granted,
                owner("github-pr"),
                mount("/secrets", "/workspace"),
            )),
        ];
        for record in records {
            assert_eq!(
                EnforcementSnapshot::new(standing(&[&granted]), vec![record.clone()], Vec::new()),
                Err(conflict(&granted)),
                "standing row, {record:?}"
            );
            assert_eq!(
                EnforcementSnapshot::new(
                    OsState::new(),
                    vec![record.clone()],
                    vec![incomplete(&granted)],
                ),
                Err(conflict(&granted)),
                "incomplete row, {record:?}"
            );
        }
    }

    #[test]
    fn an_issued_record_with_no_standing_or_incomplete_row_is_a_fault() {
        let lost = op(broker("egressd"));
        let elsewhere = op(broker("egressd"));
        assert_eq!(
            EnforcementSnapshot::new(standing(&[&elsewhere]), vec![issued(&lost)], Vec::new()),
            Err(no_holding(&lost))
        );
        assert_eq!(
            no_holding(&lost).to_string(),
            format!(
                "backend fault: issued record broker-pid/{} has no standing or incomplete holding",
                lost.id()
            )
        );
    }

    #[test]
    fn the_first_broken_rule_in_check_order_is_the_one_reported() {
        let repeated = op(session_file("/skills/pr.md"));
        let overlapping = op(uds_socket("/run/ak/egressd.uds"));
        let twice_issued = op(proxy_map("api.github.com", "splice"));
        let disagreeing = op(mount("/code", "/workspace"));
        let lost = op(broker("egressd"));
        let disagreement = issued(&at(
            &disagreeing,
            owner("audit"),
            mount("/code", "/workspace"),
        ));
        let cases = [
            (
                standing(&[&overlapping]),
                Vec::new(),
                vec![
                    incomplete(&repeated),
                    incomplete(&repeated),
                    incomplete(&overlapping),
                ],
                conflict(&repeated),
            ),
            (
                standing(&[&overlapping, &twice_issued]),
                vec![issued(&twice_issued), issued(&twice_issued)],
                vec![incomplete(&overlapping)],
                conflict(&overlapping),
            ),
            (
                standing(&[&twice_issued, &disagreeing]),
                vec![
                    disagreement.clone(),
                    issued(&twice_issued),
                    issued(&twice_issued),
                ],
                Vec::new(),
                conflict(&twice_issued),
            ),
            (
                standing(&[&disagreeing]),
                vec![issued(&lost), disagreement],
                Vec::new(),
                conflict(&disagreeing),
            ),
        ];
        for (state, grants, unfinished, expected) in cases {
            assert_eq!(
                EnforcementSnapshot::new(state, grants, unfinished),
                Err(expected.clone()),
                "{expected:?}"
            );
        }
    }

    fn descending<T: Clone>(rows: &[T], handle: impl Fn(&T) -> crate::backend::Handle) -> Vec<T> {
        let mut sorted = rows.to_vec();
        sorted.sort_by_key(|row| std::cmp::Reverse(handle(row)));
        sorted
    }

    fn ascending<T: Clone>(rows: &[T], handle: impl Fn(&T) -> crate::backend::Handle) -> Vec<T> {
        let mut sorted = rows.to_vec();
        sorted.sort_by_key(handle);
        sorted
    }

    fn rows_of_every_class() -> (OsState, Vec<LedgerEntry>, Vec<IncompleteEffect>) {
        let capabilities = || {
            [
                session_file("/skills/pr.md"),
                uds_socket("/run/ak/egressd.uds"),
                proxy_map("api.github.com", "splice"),
                broker("egressd"),
                mount("/code", "/workspace"),
            ]
        };
        let granted: Vec<UniverseOp> = capabilities()
            .into_iter()
            .chain(capabilities())
            .map(op)
            .collect();
        let unfinished: Vec<UniverseOp> = capabilities()
            .into_iter()
            .chain(capabilities())
            .map(op)
            .collect();
        let state = standing(&granted.iter().collect::<Vec<_>>());
        (
            state,
            granted.iter().map(issued).collect(),
            unfinished.iter().map(incomplete).collect(),
        )
    }

    #[test]
    fn records_and_effects_are_sorted_by_address_so_equal_accounts_compare_equal() {
        let (state, grants, unfinished) = rows_of_every_class();
        let given = EnforcementSnapshot::new(
            state.clone(),
            descending(&grants, |entry| entry.handle),
            descending(&unfinished, IncompleteEffect::handle),
        )
        .unwrap();
        assert_eq!(given.grants(), ascending(&grants, |entry| entry.handle));
        assert_eq!(
            given.incomplete(),
            ascending(&unfinished, IncompleteEffect::handle)
        );
        assert_eq!(
            given,
            EnforcementSnapshot::new(state, grants, unfinished).unwrap()
        );
    }

    #[test]
    fn the_empty_account_is_empty_and_any_one_row_makes_an_account_not_empty() {
        let empty = EnforcementSnapshot::empty();
        assert!(empty.is_empty());
        assert!(empty.state().is_empty());
        assert!(empty.grants().is_empty());
        assert!(empty.incomplete().is_empty());
        assert_eq!(
            EnforcementSnapshot::new(OsState::new(), Vec::new(), Vec::new()),
            Ok(empty)
        );
        let held = op(mount("/code", "/workspace"));
        for account in [
            EnforcementSnapshot::new(standing(&[&held]), Vec::new(), Vec::new()),
            EnforcementSnapshot::new(standing(&[&held]), vec![issued(&held)], Vec::new()),
            EnforcementSnapshot::new(OsState::new(), Vec::new(), vec![incomplete(&held)]),
            EnforcementSnapshot::new(OsState::new(), vec![issued(&held)], vec![incomplete(&held)]),
        ] {
            assert!(!account.unwrap().is_empty());
        }
    }

    fn sample() -> (EnforcementSnapshot, Value) {
        let granted = op(mount("/code", "/workspace"));
        let applied = op(proxy_map("api.github.com", "splice"));
        let unfinished = op(broker("egressd"));
        let state = standing(&[&granted, &applied]);
        let encoded = json!({
            "state": serde_json::to_value(&state).unwrap(),
            "grants": [serde_json::to_value(issued(&granted)).unwrap()],
            "incomplete": [{ "operation": serde_json::to_value(&unfinished).unwrap() }],
        });
        let snapshot =
            EnforcementSnapshot::new(state, vec![issued(&granted)], vec![incomplete(&unfinished)])
                .unwrap();
        (snapshot, encoded)
    }

    fn decoded(value: &Value) -> [Result<EnforcementSnapshot, String>; 2] {
        [
            serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
            serde_json::from_str(&value.to_string()).map_err(|error| error.to_string()),
        ]
    }

    fn refused_with(value: &Value, expected: &str) {
        for outcome in decoded(value) {
            let error = outcome.expect_err(expected);
            assert!(
                error.contains(expected),
                "`{error}` does not say `{expected}`"
            );
        }
    }

    #[test]
    fn a_snapshot_encodes_as_its_three_collections_and_decodes_back() {
        let (snapshot, encoded) = sample();
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), encoded);
        for outcome in decoded(&encoded) {
            assert_eq!(outcome, Ok(snapshot.clone()));
        }
        let effect = &snapshot.incomplete()[0];
        let effect_json = json!({ "operation": serde_json::to_value(&effect.operation).unwrap() });
        assert_eq!(serde_json::to_value(effect).unwrap(), effect_json);
        assert_eq!(
            serde_json::from_value::<IncompleteEffect>(effect_json).unwrap(),
            *effect
        );
    }

    #[test]
    fn decoding_refuses_a_missing_unknown_or_repeated_field() {
        let (_, encoded) = sample();
        for field in ["state", "grants", "incomplete"] {
            let mut missing = encoded.clone();
            missing.as_object_mut().unwrap().remove(field);
            refused_with(&missing, &format!("missing field `{field}`"));
            let text = encoded.to_string();
            let copy = format!("\"{field}\":{}", encoded[field]);
            let repeated = format!("{{{copy},{}", &text[1..]);
            let error = serde_json::from_str::<EnforcementSnapshot>(&repeated).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("duplicate field `{field}`")),
                "{error}"
            );
        }
        let mut unknown = encoded.clone();
        unknown
            .as_object_mut()
            .unwrap()
            .insert("unknown".to_string(), json!(true));
        refused_with(&unknown, "unknown field `unknown`");
        let operation = encoded["incomplete"][0]["operation"].clone();
        for (effect, expected) in [
            (json!({}), "missing field `operation`"),
            (
                json!({ "operation": operation, "unknown": true }),
                "unknown field `unknown`",
            ),
        ] {
            let error = serde_json::from_value::<IncompleteEffect>(effect).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn decoding_refuses_every_account_the_constructor_refuses() {
        let held = op(mount("/code", "/workspace"));
        let lost = op(broker("egressd"));
        let owned_elsewhere = issued(&at(&held, owner("audit"), mount("/code", "/workspace")));
        let encode = |state: OsState, grants: Vec<LedgerEntry>, unfinished: Vec<UniverseOp>| {
            json!({
                "state": serde_json::to_value(state).unwrap(),
                "grants": serde_json::to_value(grants).unwrap(),
                "incomplete": unfinished
                    .iter()
                    .map(|op| json!({ "operation": serde_json::to_value(op).unwrap() }))
                    .collect::<Vec<_>>(),
            })
        };
        for (value, refusal) in [
            (
                encode(OsState::new(), Vec::new(), vec![held.clone(), held.clone()]),
                conflict(&held),
            ),
            (
                encode(standing(&[&held]), Vec::new(), vec![held.clone()]),
                conflict(&held),
            ),
            (
                encode(
                    standing(&[&held]),
                    vec![issued(&held), issued(&held)],
                    Vec::new(),
                ),
                conflict(&held),
            ),
            (
                encode(standing(&[&held]), vec![owned_elsewhere], Vec::new()),
                conflict(&held),
            ),
            (
                encode(standing(&[&held]), vec![issued(&lost)], Vec::new()),
                no_holding(&lost),
            ),
        ] {
            refused_with(&value, &refusal.to_string());
        }
    }

    #[test]
    fn decoding_sorts_records_and_effects_like_the_constructor() {
        let (state, grants, unfinished) = rows_of_every_class();
        let value = json!({
            "state": serde_json::to_value(&state).unwrap(),
            "grants": serde_json::to_value(descending(&grants, |entry| entry.handle)).unwrap(),
            "incomplete": serde_json::to_value(descending(&unfinished, IncompleteEffect::handle)).unwrap(),
        });
        let expected = EnforcementSnapshot::new(state, grants, unfinished).unwrap();
        for outcome in decoded(&value) {
            assert_eq!(outcome, Ok(expected.clone()));
        }
    }
}
