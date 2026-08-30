use std::collections::BTreeMap;
use std::fmt;

use plasmosome_backend::PluginId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionReq {
    Any,
    Compatible { major: u32, minor: u32 },
    Pin(Version),
}

impl VersionReq {
    pub fn matches(&self, candidate: &Version) -> bool {
        match self {
            VersionReq::Any => true,
            VersionReq::Compatible { major, minor } => {
                candidate.major == *major && candidate.minor >= *minor
            }
            VersionReq::Pin(pinned) => candidate == pinned,
        }
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionReq::Any => write!(f, "*"),
            VersionReq::Compatible { major, minor } => write!(f, "^{major}.{minor}"),
            VersionReq::Pin(v) => write!(f, "={v}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub capability: String,
    pub req: VersionReq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provision {
    pub capability: String,
    pub version: Version,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictPolicy {
    pub pins: BTreeMap<String, Version>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    NoProvider {
        capability: String,
    },
    Unsatisfiable {
        capability: String,
        candidates: Vec<Version>,
        requirements: Vec<String>,
    },
    PinNotOffered {
        capability: String,
        pin: Version,
        offered: Vec<Version>,
    },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectionError::NoProvider { capability } => {
                write!(f, "no attached provider offers `{capability}`")
            }
            SelectionError::Unsatisfiable {
                capability,
                candidates,
                requirements,
            } => write!(
                f,
                "capability `{capability}` is offered as {candidates:?} but requirements {requirements:?} cannot agree on a version"
            ),
            SelectionError::PinNotOffered {
                capability,
                pin,
                offered,
            } => {
                write!(
                    f,
                    "capability `{capability}` is pinned to {pin} but providers offer {offered:?}"
                )
            }
        }
    }
}

impl std::error::Error for SelectionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub provider: PluginId,
    pub version: Version,
}

/// Pre-registered selection policy: an explicit pin overrides; otherwise the
/// newest version satisfying every requirement wins.
pub fn select_version(
    capability: &str,
    candidates: &[Candidate],
    requirements: &[VersionReq],
    policy: &ConflictPolicy,
) -> Result<Candidate, SelectionError> {
    if candidates.is_empty() {
        return Err(SelectionError::NoProvider {
            capability: capability.to_string(),
        });
    }
    if let Some(pin) = policy.pins.get(capability) {
        let pinned = candidates.iter().find(|c| c.version == *pin);
        let Some(pinned) = pinned else {
            return Err(SelectionError::PinNotOffered {
                capability: capability.to_string(),
                pin: *pin,
                offered: offered_versions(candidates),
            });
        };
        if let Some(bad) = requirements.iter().find(|r| !r.matches(&pinned.version)) {
            return Err(SelectionError::Unsatisfiable {
                capability: capability.to_string(),
                candidates: vec![pinned.version],
                requirements: vec![bad.to_string()],
            });
        }
        return Ok(pinned.clone());
    }
    let satisfying: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| requirements.iter().all(|r| r.matches(&c.version)))
        .collect();
    if satisfying.is_empty() {
        return Err(SelectionError::Unsatisfiable {
            capability: capability.to_string(),
            candidates: offered_versions(candidates),
            requirements: requirements.iter().map(|r| r.to_string()).collect(),
        });
    }
    Ok(satisfying
        .into_iter()
        .max_by_key(|c| c.version)
        .expect("non-empty after the is_empty guard")
        .clone())
}

fn offered_versions(candidates: &[Candidate]) -> Vec<Version> {
    let mut versions: Vec<Version> = candidates.iter().map(|c| c.version).collect();
    versions.sort();
    versions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(list: &[(&str, (u32, u32, u32))]) -> Vec<Candidate> {
        list.iter()
            .map(|(provider, (major, minor, patch))| Candidate {
                provider: PluginId::from(*provider),
                version: Version::new(*major, *minor, *patch),
            })
            .collect()
    }

    #[test]
    fn newest_compatible_version_wins_by_default() {
        let policy = ConflictPolicy::default();
        let pool = candidates(&[("github-v1", (1, 4, 0)), ("github-v2", (2, 1, 0))]);
        let picked = select_version(
            "github:api",
            &pool,
            &[VersionReq::Compatible { major: 1, minor: 0 }],
            &policy,
        )
        .unwrap();
        assert_eq!(picked.provider.as_str(), "github-v1");
        assert_eq!(picked.version, Version::new(1, 4, 0));
    }

    #[test]
    fn without_a_requirement_the_newest_version_wins() {
        let policy = ConflictPolicy::default();
        let pool = candidates(&[
            ("github-v1", (1, 4, 0)),
            ("github-v2", (2, 1, 0)),
            ("github-v2-patch", (2, 1, 9)),
        ]);
        let picked = select_version("github:api", &pool, &[], &policy).unwrap();
        assert_eq!(picked.version, Version::new(2, 1, 9));
    }

    #[test]
    fn an_explicit_pin_overrides_newest_wins() {
        let mut policy = ConflictPolicy::default();
        policy
            .pins
            .insert("github:api".to_string(), Version::new(1, 4, 0));
        let pool = candidates(&[("github-v1", (1, 4, 0)), ("github-v2", (2, 1, 0))]);
        let picked = select_version("github:api", &pool, &[], &policy).unwrap();
        assert_eq!(picked.provider.as_str(), "github-v1");
    }

    #[test]
    fn a_pin_to_a_version_nobody_offers_is_a_named_error() {
        let mut policy = ConflictPolicy::default();
        policy
            .pins
            .insert("github:api".to_string(), Version::new(9, 9, 9));
        let pool = candidates(&[("github-v1", (1, 4, 0))]);
        let err = select_version("github:api", &pool, &[], &policy).unwrap_err();
        assert!(matches!(err, SelectionError::PinNotOffered { .. }));
    }

    #[test]
    fn a_diamond_with_disjoint_requirements_is_a_named_conflict() {
        let policy = ConflictPolicy::default();
        let pool = candidates(&[("github-v1", (1, 4, 0)), ("github-v2", (2, 1, 0))]);
        let err = select_version(
            "github:api",
            &pool,
            &[
                VersionReq::Compatible { major: 1, minor: 0 },
                VersionReq::Compatible { major: 2, minor: 0 },
            ],
            &policy,
        )
        .unwrap_err();
        let SelectionError::Unsatisfiable { requirements, .. } = err else {
            panic!("disjoint diamond requirements must name the conflict");
        };
        assert_eq!(requirements.len(), 2);
    }

    #[test]
    fn a_diamond_with_a_shared_compatible_version_resolves_to_the_newest_match() {
        let policy = ConflictPolicy::default();
        let pool = candidates(&[("github-v1", (1, 4, 0)), ("github-v2", (2, 1, 0))]);
        let picked = select_version(
            "github:api",
            &pool,
            &[
                VersionReq::Compatible { major: 1, minor: 0 },
                VersionReq::Any,
            ],
            &policy,
        )
        .unwrap();
        assert_eq!(
            picked.version,
            Version::new(1, 4, 0),
            "^1 and Any intersect at 1.4.0; Any must not drag selection to 2.x"
        );
    }

    #[test]
    fn a_pin_satisfying_the_requirements_breaks_the_diamond() {
        let mut policy = ConflictPolicy::default();
        policy
            .pins
            .insert("github:api".to_string(), Version::new(1, 4, 0));
        let pool = candidates(&[("github-v1", (1, 4, 0)), ("github-v2", (2, 1, 0))]);
        let picked = select_version(
            "github:api",
            &pool,
            &[
                VersionReq::Compatible { major: 1, minor: 0 },
                VersionReq::Compatible { major: 1, minor: 2 },
            ],
            &policy,
        )
        .unwrap();
        assert_eq!(picked.version, Version::new(1, 4, 0));
    }

    #[test]
    fn no_candidates_at_all_is_no_provider() {
        let policy = ConflictPolicy::default();
        let err = select_version("github:api", &[], &[VersionReq::Any], &policy).unwrap_err();
        assert!(matches!(err, SelectionError::NoProvider { .. }));
    }

    #[test]
    fn compatible_requires_the_same_major_and_at_least_the_pinned_minor() {
        let policy = ConflictPolicy::default();
        let pool = candidates(&[
            ("old", (1, 2, 0)),
            ("edge", (1, 8, 0)),
            ("next-major", (2, 0, 0)),
        ]);
        let picked = select_version(
            "github:api",
            &pool,
            &[VersionReq::Compatible { major: 1, minor: 4 }],
            &policy,
        )
        .unwrap();
        assert_eq!(
            picked.provider.as_str(),
            "edge",
            "1.2.0 is below ^1.4; 2.0.0 is a different major"
        );
    }
}
