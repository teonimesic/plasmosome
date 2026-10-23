use std::collections::BTreeSet;
use std::fmt::Debug;

use plasmosome_backend::GrantId;
use plasmosome_backend::digest::Digest;
use plasmosome_backend::guest_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn empty() -> GuestObservation {
    GuestObservation::empty(BootToken::from_bytes([7; 32]), Digest::of(b"policy"))
}

fn object() -> GuestProjection {
    GuestProjection {
        id: "object".into(),
        bindings: vec![GuestBinding {
            grant: GrantId::new(),
            admission: GuestAdmission::Active,
        }],
    }
}

fn stream() -> SocketStreamProjection {
    SocketStreamProjection {
        id: "stream".into(),
        grant: GrantId::new(),
        ends: vec![
            SocketStreamEnd {
                side: StreamSide::Accepted,
                id: "accepted-end".into(),
            },
            SocketStreamEnd {
                side: StreamSide::Relay,
                id: "relay-end".into(),
            },
        ],
    }
}

fn inventories(o: &mut GuestObservation) -> [&mut Vec<GuestProjection>; 9] {
    [
        &mut o.fs_mounts,
        &mut o.fs_handles,
        &mut o.socket_listeners,
        &mut o.network.dns_listeners,
        &mut o.network.tuns,
        &mut o.network.routes,
        &mut o.network.proxy_rules,
        &mut o.network.tcp_flows,
        &mut o.network.udp_flows,
    ]
}

fn full() -> GuestObservation {
    let mut o = empty();
    for inventory in inventories(&mut o) {
        inventory.push(object());
    }
    o.socket_streams.push(stream());
    o.software.held.push(TreeReference {
        path: "/opt/detached".into(),
        pid: u64::MAX,
        comm: "worker".into(),
        exec_id: None,
    });
    o
}

fn fault(o: &GuestObservation, expected: GuestObservationFault) {
    assert_eq!(o.validate(), Err(expected.clone()));
    let error = serde_json::from_str::<GuestObservation>(&serde_json::to_string(o).unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains(&expected.to_string()), "{error}");
    let _: &dyn std::error::Error = &expected;
}

#[test]
fn every_array_is_required() {
    let value = serde_json::to_value(empty()).unwrap();
    for (parent, fields) in [
        (
            "",
            &[
                "fs_mounts",
                "fs_handles",
                "socket_listeners",
                "socket_streams",
                "network",
                "software",
            ][..],
        ),
        (
            "/network",
            &[
                "dns_listeners",
                "tuns",
                "routes",
                "proxy_rules",
                "tcp_flows",
                "udp_flows",
            ][..],
        ),
        ("/software", &["trees", "held"][..]),
    ] {
        for field in fields {
            let mut missing = value.clone();
            missing
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(*field);
            let error = serde_json::from_value::<GuestObservation>(missing).unwrap_err();
            assert!(
                error.to_string().contains("missing field"),
                "{field}: {error}"
            );
        }
    }
}

#[test]
fn empty_observation_round_trips_and_validates() {
    let o = empty();
    assert_eq!(o.validate(), Ok(()));
    assert_eq!(
        serde_json::from_str::<GuestObservation>(&serde_json::to_string(&o).unwrap()).unwrap(),
        o
    );
    assert_eq!(o.projections().count(), 0);
    assert_eq!(o.grants().count(), 0);
}

#[test]
fn validate_refuses_empty_bindings() {
    for index in 0..9 {
        let mut o = full();
        let kind = o.projections().nth(index).unwrap().0;
        inventories(&mut o)[index][0].bindings.clear();
        fault(
            &o,
            GuestObservationFault::EmptyBindings {
                kind,
                id: "object".into(),
            },
        );
    }
}

#[test]
fn validate_refuses_empty_ids_in_every_kind() {
    for index in 0..10 {
        let mut o = full();
        let kind = o.projections().nth(index).unwrap().0;
        if index == 9 {
            o.socket_streams[0].id.clear();
        } else {
            inventories(&mut o)[index][0].id.clear();
        }
        fault(&o, GuestObservationFault::EmptyId { kind });
    }
}

#[test]
fn validate_refuses_a_repeated_id_within_a_kind() {
    assert_eq!(full().validate(), Ok(()));
    for index in 0..10 {
        let mut o = full();
        let kind = o.projections().nth(index).unwrap().0;
        if index == 9 {
            o.socket_streams.push(o.socket_streams[0].clone());
        } else {
            let rows = &mut inventories(&mut o)[index];
            rows.push(rows[0].clone());
        }
        let id = if index == 9 { "stream" } else { "object" };
        fault(
            &o,
            GuestObservationFault::DuplicateObject {
                kind,
                id: id.into(),
            },
        );
    }
    let mut o = full();
    o.socket_streams[0].id = "object".into();
    assert_eq!(o.validate(), Ok(()));
}

#[test]
fn validate_refuses_a_repeated_grant_on_one_object() {
    for index in 0..9 {
        let mut o = full();
        let kind = o.projections().nth(index).unwrap().0;
        let row = &mut inventories(&mut o)[index][0];
        let binding = row.bindings[0].clone();
        row.bindings.push(binding.clone());
        fault(
            &o,
            GuestObservationFault::DuplicateBinding {
                kind,
                id: "object".into(),
                grant: binding.grant,
            },
        );
    }
    let mut o = full();
    let grant = o.fs_mounts[0].bindings[0].grant;
    o.fs_handles[0].bindings[0].grant = grant;
    o.socket_streams[0].grant = grant;
    o.fs_mounts[0].bindings.push(GuestBinding {
        grant: GrantId::new(),
        admission: GuestAdmission::Staged,
    });
    assert_eq!(o.validate(), Ok(()));
    assert_eq!(o.grants().filter(|row| row.2 == grant).count(), 3);
    assert_eq!(o.grants().count(), 11);
}

#[test]
fn stream_end_rules() {
    for count in [0, 3] {
        let mut o = full();
        o.socket_streams[0].ends = vec![o.socket_streams[0].ends[0].clone(); count];
        fault(
            &o,
            GuestObservationFault::StreamEndCount {
                id: "stream".into(),
                count,
            },
        );
    }
    for side in [StreamSide::Accepted, StreamSide::Relay] {
        let mut o = full();
        o.socket_streams[0].ends[0].side = side;
        o.socket_streams[0].ends[1].side = side;
        fault(
            &o,
            GuestObservationFault::DuplicateStreamSide {
                id: "stream".into(),
                side,
            },
        );
        o.socket_streams[0].ends.pop();
        assert_eq!(o.validate(), Ok(()));
    }
    let mut o = full();
    o.socket_streams[0].ends[0].id.clear();
    fault(
        &o,
        GuestObservationFault::EmptyStreamEndId {
            id: "stream".into(),
        },
    );
    let mut o = full();
    o.socket_streams[0].ends[1].id = "accepted-end".into();
    fault(
        &o,
        GuestObservationFault::DuplicateStreamEnd {
            end: "accepted-end".into(),
        },
    );
    let mut o = full();
    let mut second = stream();
    second.id = "second".into();
    o.socket_streams.push(second);
    fault(
        &o,
        GuestObservationFault::DuplicateStreamEnd {
            end: "accepted-end".into(),
        },
    );
    o.socket_streams[1].ends[0].id = "second-accepted".into();
    o.socket_streams[1].ends[1].id = "second-relay".into();
    assert_eq!(o.validate(), Ok(()));
}

#[test]
fn admission_names() {
    for (name, admission) in [
        ("staged", GuestAdmission::Staged),
        ("active", GuestAdmission::Active),
        ("draining", GuestAdmission::Draining),
        ("closed", GuestAdmission::Closed),
        ("revoked", GuestAdmission::Revoked),
    ] {
        assert_eq!(serde_json::to_value(admission).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<GuestAdmission>(json!(name)).unwrap(),
            admission
        );
        assert!(serde_json::from_value::<GuestAdmission>(json!({name:null})).is_err());
    }
    for name in ["open", "Active", ""] {
        assert!(serde_json::from_value::<GuestAdmission>(json!(name)).is_err());
    }
    for (name, side) in [
        ("accepted", StreamSide::Accepted),
        ("relay", StreamSide::Relay),
    ] {
        assert_eq!(serde_json::to_value(side).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<StreamSide>(json!(name)).unwrap(),
            side
        );
        assert!(serde_json::from_value::<StreamSide>(json!({name:null})).is_err());
    }
    for name in ["Accepted", "other", ""] {
        assert!(serde_json::from_value::<StreamSide>(json!(name)).is_err());
    }
    for value in [json!(null), json!(3), json!(true), json!([])] {
        assert!(serde_json::from_value::<GuestAdmission>(value.clone()).is_err());
        assert!(serde_json::from_value::<StreamSide>(value).is_err());
    }
}

#[test]
fn decoding_runs_validate() {
    let mut o = full();
    o.fs_mounts.push(o.fs_mounts[0].clone());
    fault(
        &o,
        GuestObservationFault::DuplicateObject {
            kind: ProjectionKind::FsMounts,
            id: "object".into(),
        },
    );
}

#[test]
fn projections_and_grants_cover_every_kind() {
    let o = full();
    let expected = BTreeSet::from([
        ProjectionKind::FsMounts,
        ProjectionKind::FsHandles,
        ProjectionKind::SocketListeners,
        ProjectionKind::SocketStreams,
        ProjectionKind::DnsListeners,
        ProjectionKind::Tuns,
        ProjectionKind::Routes,
        ProjectionKind::ProxyRules,
        ProjectionKind::TcpFlows,
        ProjectionKind::UdpFlows,
    ]);
    let projections: Vec<_> = o.projections().collect();
    let grants: Vec<_> = o.grants().collect();
    assert_eq!(projections.len(), 10);
    assert_eq!(grants.len(), 10);
    assert_eq!(
        projections.iter().map(|row| row.0).collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(
        grants.iter().map(|row| row.0).collect::<BTreeSet<_>>(),
        expected
    );
    for ((kind, projection), (grant_kind, id, grant)) in projections.into_iter().zip(grants) {
        assert_eq!(kind, grant_kind);
        assert_eq!(projection.id(), id);
        match projection {
            ProjectionRef::Object(p) => assert_eq!(p.bindings[0].grant, grant),
            ProjectionRef::Stream(p) => assert_eq!(p.grant, grant),
        }
    }
    assert_eq!(
        serde_json::from_str::<GuestObservation>(&serde_json::to_string(&o).unwrap()).unwrap(),
        o
    );
}

fn strict_record<T: Serialize + DeserializeOwned + PartialEq + Debug>(record: &T, fields: &[&str]) {
    let value = serde_json::to_value(record).unwrap();
    assert_eq!(&serde_json::from_value::<T>(value.clone()).unwrap(), record);
    let positional = Value::Array(fields.iter().map(|field| value[*field].clone()).collect());
    assert!(
        serde_json::from_value::<T>(positional.clone()).is_err(),
        "{positional}"
    );
    assert!(
        serde_json::from_str::<T>(&positional.to_string()).is_err(),
        "{positional}"
    );
    for field in fields {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(*field);
        assert!(
            serde_json::from_value::<T>(missing).is_err(),
            "missing {field}"
        );
        let text = value.to_string();
        let duplicate = format!(
            "{},\"{field}\":{}}}",
            &text[..text.len() - 1],
            value[*field]
        );
        assert!(
            serde_json::from_str::<T>(&duplicate).is_err(),
            "{duplicate}"
        );
    }
    let mut unknown = value;
    unknown
        .as_object_mut()
        .unwrap()
        .insert("unknown".into(), json!(true));
    assert!(serde_json::from_value::<T>(unknown).is_err());
}

#[test]
fn records_require_objects_and_exact_fields() {
    let o = full();
    strict_record(
        &o,
        &[
            "boot",
            "policy",
            "fs_mounts",
            "fs_handles",
            "socket_listeners",
            "socket_streams",
            "network",
            "software",
        ],
    );
    strict_record(
        &o.network,
        &[
            "dns_listeners",
            "tuns",
            "routes",
            "proxy_rules",
            "tcp_flows",
            "udp_flows",
        ],
    );
    strict_record(&o.fs_mounts[0], &["id", "bindings"]);
    strict_record(&o.fs_mounts[0].bindings[0], &["grant", "admission"]);
    strict_record(&o.socket_streams[0], &["id", "grant", "ends"]);
    strict_record(&o.socket_streams[0].ends[0], &["side", "id"]);
    strict_record(&o.software, &["trees", "held"]);
    strict_record(&o.software.held[0], &["path", "pid", "comm", "exec_id"]);
}

#[test]
fn software_reservation_and_tree_reference_types() {
    for element in [
        json!({}),
        json!({"path":"/opt/tree"}),
        json!("tree"),
        json!([]),
        json!(null),
        json!(0),
        json!(false),
    ] {
        let mut value = serde_json::to_value(empty()).unwrap();
        value["software"]["trees"] = json!([element]);
        let error = serde_json::from_value::<GuestObservation>(value)
            .unwrap_err()
            .to_string();
        assert!(error.contains("spec 027 reserves SoftwareTree"), "{error}");
    }
    for value in [json!({}), json!(null), json!(""), json!(0)] {
        let mut o = serde_json::to_value(empty()).unwrap();
        o["software"]["trees"] = value;
        assert!(serde_json::from_value::<GuestObservation>(o).is_err());
    }
    let reference = full().software.held.remove(0);
    for exec_id in [json!(null), json!("exec-1")] {
        let mut value = serde_json::to_value(&reference).unwrap();
        value["exec_id"] = exec_id.clone();
        let decoded: TreeReference = serde_json::from_value(value).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap()["exec_id"], exec_id);
    }
    for (field, values) in [
        ("path", vec![json!(null), json!(4)]),
        ("pid", vec![json!(-1), json!(1.5), json!("1"), json!(null)]),
        ("comm", vec![json!(null), json!(4)]),
        (
            "exec_id",
            vec![json!(0), json!(false), json!({}), json!([])],
        ),
    ] {
        for invalid in values {
            let mut value = serde_json::to_value(&reference).unwrap();
            value[field] = invalid;
            assert!(
                serde_json::from_value::<TreeReference>(value).is_err(),
                "{field}"
            );
        }
    }
}
