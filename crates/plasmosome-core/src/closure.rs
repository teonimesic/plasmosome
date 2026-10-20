#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use plasmosome_backend::PluginId;
    use proptest::prelude::*;

    use super::{AttachmentGraph, GraphError};

    const NAMED: bool = true;
    const UNNAMED: bool = false;
    const MOST_PLUGINS: usize = 8;

    fn id(name: &str) -> PluginId {
        PluginId::from(name)
    }

    fn ids(names: &[&str]) -> Vec<PluginId> {
        names.iter().copied().map(id).collect()
    }

    fn set(names: &[&str]) -> BTreeSet<PluginId> {
        names.iter().copied().map(id).collect()
    }

    fn graph(attachments: &[(&str, &[&str], bool)]) -> AttachmentGraph {
        let mut graph = AttachmentGraph::new();
        for (plugin, providers, named) in attachments {
            graph
                .attach(id(plugin), set(providers), *named)
                .expect("each test attaches providers before their requirers");
        }
        graph
    }

    fn closure(graph: &AttachmentGraph, root: &str) -> Vec<PluginId> {
        graph
            .removal_closure(&id(root))
            .expect("each test removes an attached plugin")
    }

    #[test]
    fn a_leaf_takes_only_itself() {
        let graph = graph(&[("L", &[], NAMED), ("U", &[], UNNAMED)]);

        assert!(graph.contains(&id("L")));
        assert!(!graph.contains(&id("M")));
        assert_eq!(closure(&graph, "L"), ids(&["L"]));
        assert_eq!(graph.stranded_by(&id("L")), Ok(vec![]));
    }

    #[test]
    fn an_orphaned_provider_goes_with_its_last_requirer() {
        let graph = graph(&[("P", &[], UNNAMED), ("R", &["P"], NAMED)]);

        assert_eq!(closure(&graph, "R"), ids(&["P", "R"]));
    }

    #[test]
    fn a_shared_provider_stays() {
        let graph = graph(&[
            ("P", &[], UNNAMED),
            ("R", &["P"], NAMED),
            ("Q", &["P"], NAMED),
        ]);

        assert_eq!(closure(&graph, "R"), ids(&["R"]));
    }

    #[test]
    fn a_named_provider_stays() {
        let direct = graph(&[("P", &[], NAMED), ("R", &["P"], NAMED)]);
        assert_eq!(closure(&direct, "R"), ids(&["R"]));

        let behind_a_named_one = graph(&[
            ("Q", &[], UNNAMED),
            ("P", &["Q"], NAMED),
            ("R", &["P"], NAMED),
        ]);
        assert_eq!(closure(&behind_a_named_one, "R"), ids(&["R"]));
    }

    #[test]
    fn orphans_are_followed_transitively() {
        let chain = graph(&[
            ("Q", &[], UNNAMED),
            ("P", &["Q"], UNNAMED),
            ("R", &["P"], NAMED),
        ]);
        assert_eq!(closure(&chain, "R"), ids(&["P", "Q", "R"]));

        let shared_at_the_end = graph(&[
            ("Q", &[], UNNAMED),
            ("P", &["Q"], UNNAMED),
            ("R", &["P"], NAMED),
            ("S", &["Q"], NAMED),
        ]);
        assert_eq!(closure(&shared_at_the_end, "R"), ids(&["P", "R"]));

        let deepest_sorts_first = graph(&[
            ("A", &[], UNNAMED),
            ("B", &["A"], UNNAMED),
            ("C", &["B"], NAMED),
        ]);
        assert_eq!(closure(&deepest_sorts_first, "C"), ids(&["A", "B", "C"]));

        let diamond = graph(&[
            ("Q", &[], UNNAMED),
            ("P1", &["Q"], UNNAMED),
            ("P2", &["Q"], UNNAMED),
            ("R", &["P1", "P2"], NAMED),
        ]);
        assert_eq!(closure(&diamond, "R"), ids(&["P1", "P2", "Q", "R"]));
    }

    #[test]
    fn stranded_requirers_are_all_named_and_sorted() {
        let graph = graph(&[
            ("P", &[], NAMED),
            ("Z", &["P"], NAMED),
            ("A", &["P"], NAMED),
            ("W", &["A"], NAMED),
        ]);

        assert_eq!(graph.stranded_by(&id("P")), Ok(ids(&["A", "Z"])));
        assert_eq!(graph.stranded_by(&id("A")), Ok(ids(&["W"])));
        assert_eq!(graph.stranded_by(&id("Z")), Ok(vec![]));
    }

    #[test]
    fn an_expiry_set_holds_the_requirers() {
        let direct = graph(&[("P", &[], NAMED), ("R", &["P"], NAMED)]);
        assert_eq!(closure(&direct, "P"), ids(&["P", "R"]));

        let wider = graph(&[
            ("P", &[], NAMED),
            ("T", &[], UNNAMED),
            ("Q", &[], UNNAMED),
            ("N", &[], NAMED),
            ("R", &["N", "P", "Q", "T"], NAMED),
            ("S", &["Q"], NAMED),
            ("W", &["R"], NAMED),
        ]);
        assert_eq!(closure(&wider, "P"), ids(&["P", "R", "T", "W"]));
    }

    #[test]
    fn refusals() {
        let mut graph = graph(&[("P", &[], UNNAMED), ("R", &["P"], NAMED)]);

        assert_eq!(
            graph.removal_closure(&id("X")),
            Err(GraphError::NotAttached { plugin: id("X") })
        );
        assert_eq!(
            graph.stranded_by(&id("X")),
            Err(GraphError::NotAttached { plugin: id("X") })
        );

        assert_eq!(
            graph.attach(id("S"), set(&["P", "M"]), NAMED),
            Err(GraphError::UnknownProvider {
                plugin: id("S"),
                provider: id("M"),
            })
        );
        assert!(!graph.contains(&id("S")));
        assert_eq!(graph.stranded_by(&id("P")), Ok(ids(&["R"])));

        assert_eq!(
            graph.attach(id("C"), set(&["C"]), NAMED),
            Err(GraphError::UnknownProvider {
                plugin: id("C"),
                provider: id("C"),
            })
        );
        assert!(!graph.contains(&id("C")));

        assert_eq!(
            graph.attach(id("P"), set(&[]), NAMED),
            Err(GraphError::AlreadyAttached { plugin: id("P") })
        );
        assert_eq!(
            graph.attach(id("R"), set(&[]), NAMED),
            Err(GraphError::AlreadyAttached { plugin: id("R") })
        );
        assert_eq!(graph.stranded_by(&id("P")), Ok(ids(&["R"])));
        assert_eq!(closure(&graph, "R"), ids(&["P", "R"]));
    }

    #[test]
    fn refusals_name_the_plugins() {
        let refusals: [(Box<dyn std::error::Error>, &str); 3] = [
            (
                Box::new(GraphError::NotAttached { plugin: id("X") }),
                "plugin X is not attached",
            ),
            (
                Box::new(GraphError::UnknownProvider {
                    plugin: id("S"),
                    provider: id("M"),
                }),
                "plugin S requires M, which is not attached",
            ),
            (
                Box::new(GraphError::AlreadyAttached { plugin: id("P") }),
                "plugin P is already attached",
            ),
        ];

        for (refusal, text) in refusals {
            assert_eq!(refusal.to_string(), text);
        }
    }

    type Attachment = (PluginId, BTreeSet<PluginId>, bool);

    fn attachments() -> impl Strategy<Value = Vec<Attachment>> {
        (1..=MOST_PLUGINS)
            .prop_flat_map(|count| {
                (
                    Just((0..count).collect::<Vec<usize>>()).prop_shuffle(),
                    prop::collection::vec(
                        (prop::collection::vec(any::<bool>(), count), any::<bool>()),
                        count,
                    ),
                )
            })
            .prop_map(|(names, rows)| {
                let name = |index: usize| PluginId::from(format!("p{}", names[index]));
                rows.iter()
                    .enumerate()
                    .map(|(index, (requires, named))| {
                        let providers = (0..index)
                            .filter(|earlier| requires[*earlier])
                            .map(name)
                            .collect();
                        (name(index), providers, *named)
                    })
                    .collect()
            })
    }

    fn requirers(
        edges: &BTreeMap<PluginId, BTreeSet<PluginId>>,
        provider: &PluginId,
    ) -> BTreeSet<PluginId> {
        edges
            .iter()
            .filter(|(_, providers)| providers.contains(provider))
            .map(|(plugin, _)| plugin.clone())
            .collect()
    }

    fn requires_transitively(
        edges: &BTreeMap<PluginId, BTreeSet<PluginId>>,
        plugin: &PluginId,
        root: &PluginId,
    ) -> bool {
        edges[plugin]
            .iter()
            .any(|provider| provider == root || requires_transitively(edges, provider, root))
    }

    proptest! {
        #[test]
        fn no_removal_strands_or_orphans(attachments in attachments()) {
            let mut graph = AttachmentGraph::new();
            for (plugin, providers, named) in &attachments {
                graph
                    .attach(plugin.clone(), providers.clone(), *named)
                    .expect("the strategy attaches providers first");
            }
            let edges: BTreeMap<PluginId, BTreeSet<PluginId>> = attachments
                .iter()
                .map(|(plugin, providers, _)| (plugin.clone(), providers.clone()))
                .collect();
            let named: BTreeSet<PluginId> = attachments
                .iter()
                .filter(|(_, _, named)| *named)
                .map(|(plugin, _, _)| plugin.clone())
                .collect();

            for root in edges.keys() {
                let stranded = graph.stranded_by(root).expect("the root is attached");
                prop_assert_eq!(
                    &stranded,
                    &requirers(&edges, root).into_iter().collect::<Vec<_>>()
                );

                let list = graph.removal_closure(root).expect("the root is attached");
                prop_assert!(list.windows(2).all(|pair| pair[0] < pair[1]));
                let removed: BTreeSet<PluginId> = list.into_iter().collect();
                prop_assert!(removed.contains(root));

                for (plugin, providers) in &edges {
                    let required_by = requirers(&edges, plugin);
                    if !removed.contains(plugin) {
                        prop_assert!(providers.is_disjoint(&removed));
                        if !named.contains(plugin) && !required_by.is_empty() {
                            prop_assert!(!required_by.is_subset(&removed));
                        }
                    } else if plugin != root {
                        if stranded.is_empty() {
                            prop_assert!(!named.contains(plugin));
                        }
                        prop_assert!(
                            requires_transitively(&edges, plugin, root)
                                || (!named.contains(plugin)
                                    && !required_by.is_empty()
                                    && required_by.is_subset(&removed))
                        );
                    }
                }
            }
        }
    }
}
