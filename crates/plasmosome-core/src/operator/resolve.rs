use std::net::IpAddr;

use plasmosome_backend::{Capability, GrantId, MockMode, MountRecipe, RecipeError};

use super::file::{OperatorEntry, ProxyEntry};
use crate::manifest::{NetworkSpec, PlasmidManifest, toml_string};

/// One complete, owned capability and its fresh identity, chosen before prepare.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) id: GrantId,
    pub(crate) capability: Capability,
}

/// A member refusal for unit H to map to the wire: `Unmocked` is an author 108 at `mock.hosts`,
/// `Uncovered` is 103 without a fix, and `Invalid` is an operator-file 108 at `content`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MemberRefusal {
    Unmocked {
        plasmid: String,
        fix: String,
        host: String,
    },
    Uncovered {
        plasmid: String,
        capability: String,
    },
    Invalid {
        plasmid: String,
        error: RecipeError,
    },
}

/// Resolves a parsed manifest at `mode`, using its own `OperatorFile::entry(&manifest.id)` or
/// `None`. The entry must come from `OperatorFile::parse`, including its UTF-8 source paths.
/// Returns complete capabilities with fresh IDs, or one refusal and no partial result. Reads
/// no files or cell state; callers check source safety and closure conflicts before prepare.
pub(crate) fn resolve_member(
    manifest: &PlasmidManifest,
    entry: Option<&OperatorEntry>,
    mode: MockMode,
) -> Result<Vec<Resolved>, MemberRefusal> {
    if let Some(command) = manifest.commands.as_ref().and_then(|c| c.commands.first()) {
        return Err(uncovered(
            manifest,
            format!("unsupported:commands:{}", command.id),
        ));
    }
    if let Some(secret) = manifest.secrets.first() {
        return Err(uncovered(
            manifest,
            format!("unsupported:credential:{}", secret.id),
        ));
    }
    if let Some(model) = &manifest.model {
        return Err(uncovered(
            manifest,
            format!("unsupported:credential:{}", model.credential),
        ));
    }
    let mut resolved = Vec::new();
    if let Some(network) = &manifest.network {
        if mode != MockMode::Passthrough {
            for host in &network.hosts {
                if !manifest
                    .mock
                    .as_ref()
                    .is_some_and(|m| m.hosts.contains(host))
                {
                    let quoted = toml_string(host);
                    let fix = if manifest.mock.is_some() {
                        quoted
                    } else {
                        format!("[mock]\nhosts = [{quoted}]")
                    };
                    return Err(MemberRefusal::Unmocked {
                        plasmid: manifest.id.clone(),
                        fix,
                        host: host.clone(),
                    });
                }
            }
        }
        for host in &network.hosts {
            for port in &network.ports {
                let mut covered = false;
                for proxy in entry.into_iter().flat_map(|e| &e.proxy_maps) {
                    if proxy.host != *host
                        || proxy.recipe.port != *port
                        || !proxy.modes.contains(&mode)
                        || !within_pins(network, proxy, mode)
                    {
                        continue;
                    }
                    push(
                        &mut resolved,
                        manifest,
                        Capability::ProxyMap {
                            host: proxy.host.clone(),
                            route: proxy.route.clone(),
                            recipe: proxy.recipe.clone(),
                        },
                    )?;
                    covered = true;
                }
                if !covered {
                    return Err(uncovered(manifest, format!("entry:network:{host}:{port}")));
                }
            }
        }
    }
    if let Some(workspace) = &manifest.workspace {
        let mount = entry
            .and_then(|e| e.mounts.iter().find(|m| m.target == workspace.dst))
            .ok_or_else(|| uncovered(manifest, format!("entry:workspace:{}", workspace.dst)))?;
        push(
            &mut resolved,
            manifest,
            Capability::Mount {
                source: mount
                    .source
                    .to_str()
                    .expect("operator paths are parsed from JSON strings")
                    .to_owned(),
                target: mount.target.clone(),
                recipe: MountRecipe {
                    access: mount.access,
                },
            },
        )?;
    }
    Ok(resolved)
}

fn within_pins(network: &NetworkSpec, proxy: &ProxyEntry, mode: MockMode) -> bool {
    mode != MockMode::Passthrough
        || network.pin_cidrs.is_empty()
        || proxy
            .recipe
            .destination
            .parse::<IpAddr>()
            .is_ok_and(|address| network.pin_cidrs.iter().any(|pin| pin.contains(address)))
}

fn uncovered(manifest: &PlasmidManifest, capability: String) -> MemberRefusal {
    MemberRefusal::Uncovered {
        plasmid: manifest.id.clone(),
        capability,
    }
}

fn push(
    resolved: &mut Vec<Resolved>,
    manifest: &PlasmidManifest,
    capability: Capability,
) -> Result<(), MemberRefusal> {
    capability
        .validate()
        .map_err(|error| MemberRefusal::Invalid {
            plasmid: manifest.id.clone(),
            error,
        })?;
    resolved.push(Resolved {
        id: GrantId::new(),
        capability,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use plasmosome_backend::{FileAccess, ProxyRecipe, ProxyTransport};
    use serde_json::{Value, json};

    use super::*;
    use crate::operator::file::OperatorFile;

    fn manifest(sections: &str) -> PlasmidManifest {
        PlasmidManifest::parse(&format!(
            "id = \"p\"\ndescription = \"Resolve one member.\"\nimpl.wasm = \"p.wasm\"\n{sections}"
        ))
        .expect("a valid declaration")
    }

    fn file(proxies: Vec<Value>, mounts: Vec<Value>) -> OperatorFile {
        let bytes = serde_json::to_vec(&json!({"version": 1, "plasmids": {
            "p": {"proxy_maps": proxies, "mounts": mounts}
        }}))
        .unwrap();
        OperatorFile::parse(&bytes).expect("a valid operator file")
    }

    fn proxy(host: &str, port: u16, modes: &[&str], destination: &str) -> Value {
        json!({"host": host, "port": port, "modes": modes, "route": " Route ",
            "transport": "tcp", "destination": destination, "allow_private": false})
    }

    fn mount(target: &str, access: &str) -> Value {
        json!({"target": target, "source": "/Src Path/project", "access": access})
    }

    fn capabilities(resolved: &[Resolved]) -> Vec<Capability> {
        resolved.iter().map(|r| r.capability.clone()).collect()
    }

    fn missing(capability: &str) -> MemberRefusal {
        MemberRefusal::Uncovered {
            plasmid: "p".into(),
            capability: capability.into(),
        }
    }

    fn resolve(
        manifest: &PlasmidManifest,
        file: &OperatorFile,
        mode: MockMode,
    ) -> Result<Vec<Resolved>, MemberRefusal> {
        resolve_member(manifest, file.entry(&manifest.id), mode)
    }

    fn network() -> PlasmidManifest {
        manifest(
            "[network]\nhosts = [\"api.example\"]\nports = [443]\n[mock]\nhosts = [\"api.example\"]",
        )
    }

    #[test]
    fn four_proxy_maps_keep_every_field_and_get_fresh_ids() {
        let manifest =
            manifest("[network]\nhosts = [\"a.example\", \"b.example\"]\nports = [443, 8443]");
        let mut proxies = Vec::new();
        let mut expected = Vec::new();
        for (index, (host, port)) in [
            ("a.example", 443),
            ("a.example", 8443),
            ("b.example", 443),
            ("b.example", 8443),
        ]
        .into_iter()
        .enumerate()
        {
            let transport = if index == 1 {
                ProxyTransport::Udp
            } else {
                ProxyTransport::Tcp
            };
            let destination = format!("192.0.2.{}", index + 10);
            let route = format!(" route {index} ");
            let allow_private = index == 2;
            let mut entry = proxy(host, port, &["passthrough"], &destination);
            entry["route"] = json!(route);
            entry["transport"] = json!(transport);
            entry["allow_private"] = json!(allow_private);
            proxies.push(entry);
            expected.push(Capability::ProxyMap {
                host: host.into(),
                route,
                recipe: ProxyRecipe {
                    transport,
                    destination,
                    port,
                    allow_private,
                },
            });
        }
        let file = file(proxies.clone(), vec![]);
        let first = resolve(&manifest, &file, MockMode::Passthrough).unwrap();
        assert_eq!(capabilities(&first), expected);
        assert!(first.iter().all(|r| r.capability.validate().is_ok()));
        let second = resolve(&manifest, &file, MockMode::Passthrough).unwrap();
        let ids: BTreeSet<_> = first.iter().chain(&second).map(|r| r.id).collect();
        assert_eq!(ids.len(), 8);
        for (index, capability) in expected.iter().enumerate() {
            let mut absent = proxies.clone();
            absent.remove(index);
            let Capability::ProxyMap { host, recipe, .. } = capability else {
                unreachable!()
            };
            assert_eq!(
                resolve(
                    &manifest,
                    &super::tests::file(absent, vec![]),
                    MockMode::Passthrough
                ),
                Err(missing(&format!("entry:network:{host}:{}", recipe.port)))
            );
        }
    }

    #[test]
    fn every_matching_transport_is_picked_and_undeclared_entries_are_ignored() {
        let mut udp = proxy("api.example", 443, &["capture"], "192.0.2.11");
        udp["transport"] = json!("udp");
        udp["route"] = json!("udp-route");
        let file = file(
            vec![
                proxy("api.example", 443, &["capture"], "192.0.2.10"),
                udp,
                proxy("extra.example", 443, &["capture"], "192.0.2.12"),
                proxy("api.example", 8443, &["capture"], "192.0.2.13"),
            ],
            vec![mount("/unused", "read_write")],
        );
        let result = resolve(&network(), &file, MockMode::Capture).unwrap();
        assert_eq!(
            capabilities(&result),
            vec![
                Capability::ProxyMap {
                    host: "api.example".into(),
                    route: " Route ".into(),
                    recipe: ProxyRecipe {
                        transport: ProxyTransport::Tcp,
                        destination: "192.0.2.10".into(),
                        port: 443,
                        allow_private: false
                    }
                },
                Capability::ProxyMap {
                    host: "api.example".into(),
                    route: "udp-route".into(),
                    recipe: ProxyRecipe {
                        transport: ProxyTransport::Udp,
                        destination: "192.0.2.11".into(),
                        port: 443,
                        allow_private: false
                    }
                },
            ]
        );
    }

    #[test]
    fn mode_picks_only_its_entries_and_never_falls_back() {
        let live = proxy("api.example", 443, &["passthrough"], "192.0.2.10");
        let mut mock = proxy("api.example", 443, &["simulate"], "127.0.0.1");
        mock["allow_private"] = json!(true);
        let file = file(vec![live, mock], vec![]);
        for (mode, destination, allow_private) in [
            (MockMode::Passthrough, "192.0.2.10", false),
            (MockMode::Simulate, "127.0.0.1", true),
        ] {
            let result = resolve(&network(), &file, mode).unwrap();
            assert_eq!(
                capabilities(&result),
                [Capability::ProxyMap {
                    host: "api.example".into(),
                    route: " Route ".into(),
                    recipe: ProxyRecipe {
                        transport: ProxyTransport::Tcp,
                        destination: destination.into(),
                        port: 443,
                        allow_private
                    },
                }]
            );
        }
        assert_eq!(
            resolve(&network(), &file, MockMode::Capture),
            Err(missing("entry:network:api.example:443"))
        );
        let all = file_with_all_modes();
        for mode in [MockMode::Passthrough, MockMode::Simulate, MockMode::Capture] {
            assert_eq!(resolve(&network(), &all, mode).unwrap().len(), 1);
        }
    }

    fn file_with_all_modes() -> OperatorFile {
        file(
            vec![proxy(
                "api.example",
                443,
                &["simulate", "capture", "passthrough"],
                "192.0.2.10",
            )],
            vec![],
        )
    }

    #[test]
    fn pins_require_an_ip_in_a_matching_family_only_in_passthrough() {
        for (pin, destination, eligible) in [
            ("192.0.2.0/25", "192.0.2.10", true),
            ("192.0.2.0/25", "192.0.2.0", true),
            ("192.0.2.0/25", "192.0.2.127", true),
            ("192.0.2.0/25", "192.0.2.128", false),
            ("192.0.2.0/25", "192.0.2.200", false),
            ("192.0.2.0/25", "192.0.2.10.example.com", false),
            ("192.0.2.0/25", "2001:db8::1", false),
            ("2001:db8::/48", "2001:db8::1", true),
            ("2001:db8::/48", "2001:db8:1::1", false),
            ("2001:db8::/48", "192.0.2.10", false),
        ] {
            let mut declaration = network();
            declaration.network.as_mut().unwrap().pin_cidrs =
                vec![crate::manifest::Cidr::parse(pin).unwrap()];
            let file = file(
                vec![proxy("api.example", 443, &["passthrough"], destination)],
                vec![],
            );
            let result = resolve(&declaration, &file, MockMode::Passthrough);
            if eligible {
                assert_eq!(result.unwrap().len(), 1, "{pin}, {destination}");
            } else {
                assert_eq!(
                    result,
                    Err(missing("entry:network:api.example:443")),
                    "{pin}, {destination}"
                );
            }
        }
        let declaration = manifest(
            "[network]\nhosts = [\"api.example\"]\nports = [443]\npin_cidrs = [\"198.51.100.0/24\", \"192.0.2.0/25\"]\n[mock]\nhosts = [\"api.example\"]",
        );
        assert_eq!(
            resolve(&declaration, &file_with_all_modes(), MockMode::Passthrough)
                .unwrap()
                .len(),
            1
        );
        for destination in ["127.0.0.1", "MOCK.Example"] {
            let file = file(
                vec![proxy(
                    "api.example",
                    443,
                    &["simulate", "capture"],
                    destination,
                )],
                vec![],
            );
            for mode in [MockMode::Simulate, MockMode::Capture] {
                assert_eq!(resolve(&declaration, &file, mode).unwrap().len(), 1);
            }
        }
    }

    #[test]
    fn pins_filter_each_transport_without_discarding_an_eligible_peer() {
        let mut manifest = network();
        manifest.network.as_mut().unwrap().pin_cidrs =
            vec![crate::manifest::Cidr::parse("192.0.2.0/25").unwrap()];
        let mut udp = proxy("api.example", 443, &["passthrough"], "192.0.2.10");
        udp["transport"] = json!("udp");
        let file = file(
            vec![
                proxy("api.example", 443, &["passthrough"], "192.0.2.200"),
                udp,
            ],
            vec![],
        );
        let result = resolve(&manifest, &file, MockMode::Passthrough).unwrap();
        assert_eq!(result.len(), 1);
        assert!(
            matches!(&result[0].capability, Capability::ProxyMap { recipe, .. } if recipe.transport == ProxyTransport::Udp)
        );
    }

    #[test]
    fn mock_coverage_precedes_entries_and_names_the_first_unmocked_host() {
        let partial = manifest(
            "[network]\nhosts = [\"a.example\", \"b.example\"]\nports = [443]\n[mock]\nhosts = [\"a.example\"]",
        );
        let absent = manifest("[network]\nhosts = [\"a.example\", \"b.example\"]\nports = [443]");
        let file = file(
            vec![
                proxy(
                    "a.example",
                    443,
                    &["simulate", "capture", "passthrough"],
                    "192.0.2.10",
                ),
                proxy(
                    "b.example",
                    443,
                    &["simulate", "capture", "passthrough"],
                    "192.0.2.11",
                ),
            ],
            vec![],
        );
        for mode in [MockMode::Simulate, MockMode::Capture] {
            for entries in [&file, &OperatorFile::empty()] {
                assert_eq!(
                    resolve(&partial, entries, mode),
                    Err(MemberRefusal::Unmocked {
                        plasmid: "p".into(),
                        host: "b.example".into(),
                        fix: "\"b.example\"".into(),
                    })
                );
                assert_eq!(
                    resolve(&absent, entries, mode),
                    Err(MemberRefusal::Unmocked {
                        plasmid: "p".into(),
                        host: "a.example".into(),
                        fix: "[mock]\nhosts = [\"a.example\"]".into(),
                    })
                );
            }
            let mut complete = partial.clone();
            complete
                .mock
                .as_mut()
                .unwrap()
                .hosts
                .push("b.example".into());
            assert_eq!(resolve(&complete, &file, mode).unwrap().len(), 2);
        }
        for declaration in [&partial, &absent] {
            assert_eq!(
                resolve(declaration, &file, MockMode::Passthrough)
                    .unwrap()
                    .len(),
                2
            );
        }
    }

    #[test]
    fn unsupported_needs_name_the_first_command_then_secret_then_model() {
        let cases = [
            (
                "[commands.commands.first]\nexec = [\"first\"]\n[commands.commands.second]\nexec = [\"second\"]",
                "unsupported:commands:first",
            ),
            (
                "[secrets]\nrefs = [{ id = \"z/key\", consumer = \"wasm\" }, { id = \"a/key\", consumer = \"wasm\" }]",
                "unsupported:credential:z/key",
            ),
            (
                "[model]\nendpoint = \"https://api.example\"",
                "unsupported:credential:model-provider/key",
            ),
            (
                "[model]\nendpoint = \"https://api.example\"\ncredential = \"custom/key\"",
                "unsupported:credential:custom/key",
            ),
        ];
        for (sections, capability) in cases {
            for mode in [MockMode::Simulate, MockMode::Capture, MockMode::Passthrough] {
                assert_eq!(
                    resolve(&manifest(sections), &OperatorFile::empty(), mode),
                    Err(missing(capability))
                );
            }
        }
        let all = format!("{}\n{}\n{}", cases[0].0, cases[1].0, cases[2].0);
        assert_eq!(
            resolve(&manifest(&all), &OperatorFile::empty(), MockMode::Simulate),
            Err(missing(cases[0].1))
        );
        let secrets_and_model = format!("{}\n{}", cases[1].0, cases[2].0);
        assert_eq!(
            resolve(
                &manifest(&secrets_and_model),
                &OperatorFile::empty(),
                MockMode::Simulate
            ),
            Err(missing(cases[1].1))
        );
    }

    #[test]
    fn workspace_uses_only_its_exact_target_and_copies_source_and_access() {
        for (sections, target) in [
            ("[workspace]\nmount = {}", "/workspace"),
            (
                "[workspace]\nmount = { dst = \"/Work Space\", backend = \"virtiofs\" }",
                "/Work Space",
            ),
        ] {
            let manifest = manifest(sections);
            for (access, expected) in [
                ("read_only", FileAccess::ReadOnly),
                ("read_write", FileAccess::ReadWrite),
            ] {
                let file = file(
                    vec![],
                    vec![mount("/wrong", "read_write"), mount(target, access)],
                );
                for mode in [MockMode::Simulate, MockMode::Capture, MockMode::Passthrough] {
                    let result = resolve(&manifest, &file, mode).unwrap();
                    assert_eq!(
                        capabilities(&result),
                        [Capability::Mount {
                            source: "/Src Path/project".into(),
                            target: target.into(),
                            recipe: MountRecipe { access: expected },
                        }]
                    );
                }
            }
            for file in [
                OperatorFile::empty(),
                file(vec![], vec![mount("/wrong", "read_only")]),
            ] {
                assert_eq!(
                    resolve(&manifest, &file, MockMode::Passthrough),
                    Err(missing(&format!("entry:workspace:{target}")))
                );
            }
        }
    }

    #[test]
    fn network_and_workspace_return_owned_values_and_distinct_identities() {
        let mut manifest = network();
        manifest.workspace = Some(crate::manifest::WorkspaceMount {
            dst: "/workspace".into(),
        });
        let result = {
            let file = file(
                vec![proxy("api.example", 443, &["passthrough"], "192.0.2.10")],
                vec![mount("/workspace", "read_write")],
            );
            resolve(&manifest, &file, MockMode::Passthrough).unwrap()
        };
        assert_eq!(result.len(), 2);
        assert_ne!(result[0].id, result[1].id);
        assert!(matches!(result[0].capability, Capability::ProxyMap { .. }));
        assert!(matches!(result[1].capability, Capability::Mount { .. }));
    }

    #[test]
    fn no_needs_yields_nothing_even_with_operator_entries() {
        let declaration = manifest("[provides.\"p:tools\".tools]\nread = \"Read a thing.\"");
        for file in [OperatorFile::empty(), file_with_all_modes()] {
            for mode in [MockMode::Simulate, MockMode::Capture, MockMode::Passthrough] {
                assert!(resolve(&declaration, &file, mode).unwrap().is_empty());
            }
        }
        let mut empty_commands = manifest("");
        empty_commands.commands = Some(crate::manifest::CommandsSpec {
            address_plan: "10.29.0.0/24".into(),
            commands: vec![],
        });
        assert!(
            resolve_member(&empty_commands, None, MockMode::Passthrough)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn another_members_listing_never_covers_this_member() {
        let bytes = serde_json::to_vec(&json!({"version": 1, "plasmids": {
            "r": {"proxy_maps": [proxy("api.example", 443, &["passthrough"], "192.0.2.10")], "mounts": [mount("/workspace", "read_write")]}
        }})).unwrap();
        let file = OperatorFile::parse(&bytes).unwrap();
        assert_eq!(
            resolve(&network(), &file, MockMode::Passthrough),
            Err(missing("entry:network:api.example:443"))
        );
        assert_eq!(
            resolve(
                &manifest("[workspace]\nmount = {}"),
                &file,
                MockMode::Passthrough
            ),
            Err(missing("entry:workspace:/workspace"))
        );
        let p = manifest("[network]\nhosts = [\"p.example\"]\nports = [443]");
        assert_eq!(
            resolve_member(&p, file.entry("r"), MockMode::Passthrough),
            Err(missing("entry:network:p.example:443"))
        );
    }

    #[test]
    fn invalid_in_memory_capabilities_are_refused_before_they_leave() {
        let file = file_with_all_modes();
        for (kind, error) in [
            (
                "recipe",
                RecipeError::InvalidDestination {
                    value: "https://wrong.example".into(),
                },
            ),
            ("route", RecipeError::ContainsNul { field: "route" }),
        ] {
            let mut entry = file.entry("p").unwrap().clone();
            if kind == "recipe" {
                entry.proxy_maps[0].recipe.destination = "https://wrong.example".into();
            } else {
                entry.proxy_maps[0].route = "bad\0route".into();
            }
            assert_eq!(
                resolve_member(&network(), Some(&entry), MockMode::Passthrough),
                Err(MemberRefusal::Invalid {
                    plasmid: "p".into(),
                    error
                })
            );
        }
        let declaration = manifest("[workspace]\nmount = {}");
        let file = super::tests::file(vec![], vec![mount("/workspace", "read_only")]);
        let mut entry = file.entry("p").unwrap().clone();
        entry.mounts[0].source = "/src/../private".into();
        assert_eq!(
            resolve_member(&declaration, Some(&entry), MockMode::Passthrough),
            Err(MemberRefusal::Invalid {
                plasmid: "p".into(),
                error: RecipeError::NotCanonical {
                    field: "source",
                    value: "/src/../private".into()
                },
            })
        );
    }
}
