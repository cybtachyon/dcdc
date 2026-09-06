//! Resolves which version of a tool a plugin command needs, from
//! the `mise.toml` files in play.
//!
//! Three sources exist: the plugin's own `mise.toml`, the project
//! root's, and the working directory's. Which one applies depends
//! on the run target, per the plan:
//!
//! - on the host: the working directory's file wins, then the
//!   project root's, then the plugin's,
//! - in a container with the working directory bind-mounted: the
//!   container's own tools apply, which is exactly the working
//!   directory's file, because mise inside the container reads it,
//! - in a container without a mount: the project root's constraint
//!   applies, else the version the container already has.
//!
//! A missing tool with a declared version is an install offer; a
//! missing tool with no declaration is the "nothing will happen"
//! warning, and the command returns 127.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Error, Result};
use crate::plugin::runtime::Target;

/// Tool to version string, parsed from a `mise.toml`.
pub type ToolSet = BTreeMap<String, String>;

/// Where a version constraint came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cwd,
    ProjectRoot,
    Plugin,
}

impl Source {
    /// The user-facing name of the source, for messages.
    pub fn label(self) -> &'static str {
        match self {
            Source::Cwd => "the working directory's mise.toml",
            Source::ProjectRoot => "the project root's mise.toml",
            Source::Plugin => "the plugin's mise.toml",
        }
    }
}

/// The resolved policy for one tool of one invocation.
#[derive(Debug, Clone)]
pub struct ToolPolicy {
    /// The version constraint, or none when nothing declares it.
    pub spec: Option<String>,
    /// Where the constraint came from, when any.
    pub source: Option<Source>,
    /// Whether any of the three sources names the tool at all.
    pub declared: bool,
}

/// Parses a `mise.toml` into a tool set.
///
/// Reads the `[tools]` table, the current mise layout. A missing
/// file yields an empty set, so callers can probe all three
/// locations uniformly.
pub fn parse_mise(path: &Path) -> Result<ToolSet> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(BTreeMap::new());
    };
    #[derive(serde::Deserialize)]
    struct Doc {
        #[serde(default)]
        tools: BTreeMap<String, String>,
    }
    let doc: Doc = toml::from_str(&text).map_err(|e| Error::BadConfig {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(doc.tools)
}

/// A parsed version, with at least a major component.
///
/// Trailing parts beyond the patch, and pre-release or build
/// suffixes, are dropped: the comparisons of the fuzzy matchers
/// are numeric only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// Parses a version string of one to three numeric parts.
///
/// `1.7` is `1.7.0`; `1` is `1.0.0`. A string without a leading
/// number, like `latest`, does not parse.
pub fn parse_version(input: &str) -> Option<Version> {
    let s = input.trim();
    // Drop a leading `v` some tools print.
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = [0u32; 3];
    let mut count = 0;
    for piece in s.split('.') {
        if count == 3 {
            break;
        }
        // A piece must be all digits; a pre-release like
        // `1.2.3-beta` fails on the `-beta` piece and the whole
        // parse does, which is what callers want.
        let piece = piece.trim();
        let Ok(n) = piece.parse::<u32>() else {
            return None;
        };
        parts[count] = n;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    Some(Version {
        major: parts[0],
        minor: parts[1],
        patch: parts[2],
    })
}

/// The range a constraint names, as a pair of bounds.
#[derive(Debug, Clone, Copy)]
pub struct Range {
    /// Inclusive lower bound.
    pub min: Version,
    /// Exclusive upper bound, when the constraint has one.
    pub max: Option<Version>,
}

/// The fuzzy matchers of the plan, as a version range.
///
/// - `~X.Y.Z`: `>=X.Y.Z`, `<X.(Y+1).0`
/// - `^X.Y.Z`: `>=X.Y.Z`, `<(X+1).0.0`
/// - `X.Y`: `>=X.Y.0`, `<(X+1).0.0`
/// - `X`: `>=X.0.0`, `<(X+1).0.0`
/// - `latest` or `*`: any version
/// - a bare `X[.Y[.Z]]`: an exact version, when it has more than
///   one part
pub fn constraint_range(spec: &str) -> Option<Range> {
    let spec = spec.trim();
    if spec == "latest" || spec == "*" {
        return Some(Range {
            min: Version {
                major: 0,
                minor: 0,
                patch: 0,
            },
            max: None,
        });
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Op {
        Tilde,
        Caret,
        Plain,
    }
    let (op, rest) = match spec.as_bytes().first() {
        Some(b'~') => (Op::Tilde, &spec[1..]),
        Some(b'^') => (Op::Caret, &spec[1..]),
        _ => (Op::Plain, spec),
    };
    let parsed = parse_version(rest)?;
    let parts = rest.split('.').count();
    // A bare version with three parts is exact; two parts and one
    // part are floors at the minor and major level, per the plan.
    if op == Op::Plain && parts == 3 {
        return Some(Range {
            min: parsed,
            max: Some(Version {
                major: parsed.major,
                minor: parsed.minor,
                patch: parsed.patch + 1,
            }),
        });
    }
    let max = match op {
        Op::Tilde => Some(Version {
            major: parsed.major,
            minor: parsed.minor + 1,
            patch: 0,
        }),
        _ => Some(Version {
            major: parsed.major + 1,
            minor: 0,
            patch: 0,
        }),
    };
    Some(Range { min: parsed, max })
}

/// Whether a concrete version satisfies a constraint.
pub fn matches(spec: &str, version: &Version) -> bool {
    match constraint_range(spec) {
        None => false,
        Some(range) => {
            if *version < range.min {
                return false;
            }
            match range.max {
                Some(max) => *version < max,
                None => true,
            }
        }
    }
}

/// The resolved policy for `tool`, given the three mise sources and
/// the run target.
///
/// The source order follows what the target can see:
///
/// - on the host, the working directory's file, then the project
///   root's, then the plugin's,
/// - in a container with the working directory mounted, the
///   container's mise reads exactly the mounted file, so that one
///   and then the plugin's, the project file not being visible
///   inside,
/// - in a container without a mount, the project root's
///   constraint, then the plugin's, the working directory's file
///   not being visible inside either.
pub fn resolve(
    tool: &str,
    target: &Target,
    cwd: &ToolSet,
    root: &ToolSet,
    plugin: &ToolSet,
) -> ToolPolicy {
    let order: &[(Source, &ToolSet)] = if target.is_local {
        &[
            (Source::Cwd, cwd),
            (Source::ProjectRoot, root),
            (Source::Plugin, plugin),
        ]
    } else if target.cwd_mounted {
        &[(Source::Cwd, cwd), (Source::Plugin, plugin)]
    } else {
        &[(Source::ProjectRoot, root), (Source::Plugin, plugin)]
    };
    for (source, set) in order {
        if let Some(spec) = set.get(tool) {
            return ToolPolicy {
                spec: Some(spec.clone()),
                source: Some(*source),
                declared: true,
            };
        }
    }
    ToolPolicy {
        spec: None,
        source: None,
        declared: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        parse_version(s).unwrap()
    }

    #[test]
    fn parses_one_to_three_part_versions() {
        assert_eq!(
            v("1"),
            Version {
                major: 1,
                minor: 0,
                patch: 0
            }
        );
        assert_eq!(
            v("1.7"),
            Version {
                major: 1,
                minor: 7,
                patch: 0
            }
        );
        assert_eq!(
            v("v1.7.2"),
            Version {
                major: 1,
                minor: 7,
                patch: 2
            }
        );
        assert_eq!(parse_version("1.2.3.4").is_none(), false);
        assert_eq!(parse_version("latest").is_none(), true);
        assert_eq!(parse_version("1.2-beta").is_none(), true);
    }

    #[test]
    fn tilde_pins_the_major_and_bumps_the_minor() {
        assert!(matches("~1.7.2", &v("1.7.2")));
        assert!(matches("~1.7.2", &v("1.7.9")));
        assert!(!matches("~1.7.2", &v("1.8.0")));
        assert!(!matches("~1.7.2", &v("1.7.1")));
    }

    #[test]
    fn caret_pins_the_major() {
        assert!(matches("^1.7.2", &v("1.9.9")));
        assert!(!matches("^1.7.2", &v("2.0.0")));
        assert!(!matches("^1.7.2", &v("1.7.1")));
    }

    #[test]
    fn two_part_and_one_part_are_major_floors() {
        assert!(matches("1.7", &v("1.9.0")));
        assert!(!matches("1.7", &v("2.0.0")));
        assert!(!matches("1.7", &v("1.6.9")));
        assert!(matches("1", &v("1.0.0")));
        assert!(matches("1", &v("1.99.99")));
        assert!(!matches("1", &v("2.0.0")));
    }

    #[test]
    fn a_bare_exact_version_matches_only_itself() {
        assert!(matches("1.7.2", &v("1.7.2")));
        assert!(!matches("1.7.2", &v("1.7.3")));
    }

    #[test]
    fn latest_and_star_match_anything() {
        assert!(matches("latest", &v("0.1.0")));
        assert!(matches("latest", &v("99.99.99")));
        assert!(matches("*", &v("0.0.1")));
    }

    #[test]
    fn resolve_prefers_cwd_over_root_over_plugin() {
        let target = Target {
            name: "local".into(),
            is_local: true,
            workdir: "/x".into(),
            shell: "sh".into(),
            cwd_mounted: false,
        };
        let mut cwd = ToolSet::new();
        let mut root = ToolSet::new();
        let mut plugin = ToolSet::new();
        cwd.insert("jq".into(), "~1.7".into());
        root.insert("jq".into(), "^1.6".into());
        plugin.insert("jq".into(), "*".into());

        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec.as_deref(), Some("~1.7"));
        assert_eq!(policy.source, Some(Source::Cwd));

        cwd.clear();
        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec.as_deref(), Some("^1.6"));
        assert_eq!(policy.source, Some(Source::ProjectRoot));

        root.clear();
        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec.as_deref(), Some("*"));
        assert_eq!(policy.source, Some(Source::Plugin));

        plugin.clear();
        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec, None);
        assert!(!policy.declared);
    }

    #[test]
    fn a_mounted_container_reads_only_the_mounted_file_and_the_plugin() {
        let target = Target {
            name: "api".into(),
            is_local: false,
            workdir: "/app".into(),
            shell: "sh".into(),
            cwd_mounted: true,
        };
        let mut cwd = ToolSet::new();
        let mut root = ToolSet::new();
        let mut plugin = ToolSet::new();
        cwd.insert("jq".into(), "~1.7".into());
        root.insert("jq".into(), "^1.6".into());
        plugin.insert("jq".into(), "*".into());

        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec.as_deref(), Some("~1.7"));
        assert_eq!(policy.source, Some(Source::Cwd));

        // The project file is not visible inside the container, so
        // without the cwd file the plugin's declaration applies.
        cwd.clear();
        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec.as_deref(), Some("*"));
        assert_eq!(policy.source, Some(Source::Plugin));
    }

    #[test]
    fn an_unmounted_container_reads_root_and_plugin_only() {
        let target = Target {
            name: "api".into(),
            is_local: false,
            workdir: "/app".into(),
            shell: "sh".into(),
            cwd_mounted: false,
        };
        let mut cwd = ToolSet::new();
        let mut root = ToolSet::new();
        let mut plugin = ToolSet::new();
        cwd.insert("jq".into(), "~1.7".into());
        root.insert("jq".into(), "^1.6".into());
        plugin.insert("jq".into(), "*".into());

        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec.as_deref(), Some("^1.6"));
        assert_eq!(policy.source, Some(Source::ProjectRoot));

        root.clear();
        let policy = resolve("jq", &target, &cwd, &root, &plugin);
        assert_eq!(policy.spec.as_deref(), Some("*"));
        assert_eq!(policy.source, Some(Source::Plugin));
    }
}
