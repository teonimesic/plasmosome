use std::collections::BTreeSet;

use super::OperatorFile;

/// Parses any `data` without panicking, formats refusals, and checks that accepted entries
/// have no repeated ProxyMap mode for one host, port and transport, or repeated Mount target.
pub fn check(data: &[u8]) {
    let file = match OperatorFile::parse(data) {
        Ok(file) => file,
        Err(refusal) => {
            let _ = refusal.to_string();
            return;
        }
    };
    for entry in file.plasmids.values() {
        let mut served = BTreeSet::new();
        for proxy in &entry.proxy_maps {
            for mode in &proxy.modes {
                assert!(
                    served.insert((
                        proxy.host.as_str(),
                        proxy.recipe.port,
                        proxy.recipe.transport,
                        mode.as_str(),
                    )),
                    "accepted ProxyMaps share a mode for one host, port and transport"
                );
            }
        }
        let mut targets = BTreeSet::new();
        for mount in &entry.mounts {
            assert!(
                targets.insert(mount.target.as_str()),
                "accepted Mounts share a target"
            );
        }
    }
}
