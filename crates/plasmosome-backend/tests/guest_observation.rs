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
    use ProjectionKind::*;
    let mut o = empty();
    for (inventory, ids) in [
        (&mut o.fs_mounts, ["mount-0", "mount-1"]),
        (&mut o.fs_handles, ["handle-0", "handle-1"]),
        (&mut o.socket_listeners, ["listener-0", "listener-1"]),
        (&mut o.network.dns_listeners, ["dns-0", "dns-1"]),
        (&mut o.network.tuns, ["tun-0", "tun-1"]),
        (&mut o.network.routes, ["route-0", "route-1"]),
        (&mut o.network.proxy_rules, ["proxy-0", "proxy-1"]),
        (&mut o.network.tcp_flows, ["tcp-0", "tcp-1"]),
        (&mut o.network.udp_flows, ["udp-0", "udp-1"]),
    ] {
        for id in ids {
            let mut p = object();
            p.id = id.into();
            inventory.push(p);
        }
    }
    o.fs_handles[1].bindings.push(GuestBinding {
        grant: GrantId::new(),
        admission: GuestAdmission::Revoked,
    });
    for id in ["stream-0", "stream-1"] {
        let mut p = stream();
        p.id = id.into();
        p.ends[0].id = format!("{id}-accepted");
        p.ends[1].id = format!("{id}-relay");
        o.socket_streams.push(p);
    }
    o.software.held = full().software.held;
    assert_eq!(o.validate(), Ok(()));
    assert_eq!(
        o.projections()
            .map(|(kind, projection)| (kind, projection.id().to_owned()))
            .collect::<Vec<_>>(),
        [
            (FsMounts, "mount-0"),
            (FsMounts, "mount-1"),
            (FsHandles, "handle-0"),
            (FsHandles, "handle-1"),
            (SocketListeners, "listener-0"),
            (SocketListeners, "listener-1"),
            (DnsListeners, "dns-0"),
            (DnsListeners, "dns-1"),
            (Tuns, "tun-0"),
            (Tuns, "tun-1"),
            (Routes, "route-0"),
            (Routes, "route-1"),
            (ProxyRules, "proxy-0"),
            (ProxyRules, "proxy-1"),
            (TcpFlows, "tcp-0"),
            (TcpFlows, "tcp-1"),
            (UdpFlows, "udp-0"),
            (UdpFlows, "udp-1"),
            (SocketStreams, "stream-0"),
            (SocketStreams, "stream-1"),
        ]
        .map(|(kind, id)| (kind, id.to_owned()))
    );
    let grant = |p: &GuestProjection| p.bindings[0].grant;
    assert_eq!(
        o.grants().collect::<Vec<_>>(),
        [
            (FsMounts, "mount-0", grant(&o.fs_mounts[0])),
            (FsMounts, "mount-1", grant(&o.fs_mounts[1])),
            (FsHandles, "handle-0", grant(&o.fs_handles[0])),
            (FsHandles, "handle-1", grant(&o.fs_handles[1])),
            (FsHandles, "handle-1", o.fs_handles[1].bindings[1].grant),
            (SocketListeners, "listener-0", grant(&o.socket_listeners[0])),
            (SocketListeners, "listener-1", grant(&o.socket_listeners[1])),
            (DnsListeners, "dns-0", grant(&o.network.dns_listeners[0])),
            (DnsListeners, "dns-1", grant(&o.network.dns_listeners[1])),
            (Tuns, "tun-0", grant(&o.network.tuns[0])),
            (Tuns, "tun-1", grant(&o.network.tuns[1])),
            (Routes, "route-0", grant(&o.network.routes[0])),
            (Routes, "route-1", grant(&o.network.routes[1])),
            (ProxyRules, "proxy-0", grant(&o.network.proxy_rules[0])),
            (ProxyRules, "proxy-1", grant(&o.network.proxy_rules[1])),
            (TcpFlows, "tcp-0", grant(&o.network.tcp_flows[0])),
            (TcpFlows, "tcp-1", grant(&o.network.tcp_flows[1])),
            (UdpFlows, "udp-0", grant(&o.network.udp_flows[0])),
            (UdpFlows, "udp-1", grant(&o.network.udp_flows[1])),
            (SocketStreams, "stream-0", o.socket_streams[0].grant),
            (SocketStreams, "stream-1", o.socket_streams[1].grant),
        ]
    );
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
