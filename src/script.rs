use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Describes one Bash script found under `.dcdc/cmd`.
///
/// `subdir` is `None` for scripts in the root `cmd` directory, which
/// run locally, and `Some(name)` for scripts in a subdirectory of
/// `cmd`, which targets a docker compose service of that name.
#[derive(Debug)]
pub struct Script {
    pub name: String,
    pub path: PathBuf,
    pub subdir: Option<String>,
}

impl Script {
    /// Builds a script from a file path, using the file name
    /// without the `.sh` extension as the name.
    fn new(path: &Path, subdir: Option<&str>) -> Self {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        Script {
            name: name.to_string(),
            path: path.to_path_buf(),
            subdir: subdir.map(str::to_string),
        }
    }
}

/// Discovers every `.sh` script under the given `.dcdc/cmd`
/// directory, including nested subdirectories.
///
/// An empty list is returned when the directory does not exist,
/// since a project may not define any scripts yet.
pub fn load(cmd_dir: &Path) -> Result<Vec<Script>> {
    if !cmd_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut scripts = Vec::new();
    collect(cmd_dir, None, &mut scripts)?;
    // Sorted output makes the listing and ambiguity reports stable,
    // regardless of directory entry order.
    scripts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(scripts)
}

/// Finds the script whose name matches `name`, which may be
/// given with or without the `.sh` extension.
///
/// An error is returned when no script matches, or when the name
/// matches scripts in more than one location.
pub fn select<'a>(scripts: &'a [Script], name: &str) -> Result<&'a Script> {
    let want = name.strip_suffix(".sh").unwrap_or(name);
    let matches: Vec<&Script> = scripts
        .iter()
        .filter(|script| script.name == want)
        .collect();
    match matches.as_slice() {
        [] => Err(Error::ScriptNotFound {
            name: want.to_string(),
            available: scripts.iter().map(label).collect(),
        }),
        [script] => Ok(script),
        many => Err(Error::AmbiguousScript {
            name: want.to_string(),
            matches: many.iter().map(|script| label(script)).collect(),
        }),
    }
}

/// Renders a script for display: the bare name for local
/// scripts, and `name (docker: service)` for service scripts.
pub fn label(script: &Script) -> String {
    match &script.subdir {
        None => script.name.clone(),
        Some(service) => format!("{} (docker: {service})", script.name),
    }
}

/// Appends every script in one directory level to `out`,
/// recursing into subdirectories.
///
/// Nested levels inherit the top-level `subdir`, because only the
/// first level maps to a compose service name.
fn collect(dir: &Path, subdir: Option<&str>, out: &mut Vec<Script>) -> Result<()> {
    for entry in dir.read_dir()? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "sh") {
            out.push(Script::new(&path, subdir));
        } else if path.is_dir() {
            let label = subdir.or_else(|| path.file_name().and_then(|name| name.to_str()));
            collect(&path, label, out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    /// Lays out a fixture tree under a fresh temp directory:
    /// `cmd/a.sh`, `cmd/api/b.sh`, `cmd/api/deep/c.sh`, and
    /// `cmd/notes.txt`.
    ///
    /// Returns the temp directory, which the caller removes.
    fn build() -> PathBuf {
        let base = testutil::temp_subdir("scripts");
        let cmd = base.join("cmd");
        std::fs::create_dir_all(cmd.join("api").join("deep")).unwrap();
        for (name, contents) in [
            ("a.sh", "# a"),
            ("api/b.sh", "# b"),
            ("api/deep/c.sh", "# c"),
            ("notes.txt", "not a script"),
        ] {
            std::fs::write(cmd.join(name), contents).unwrap();
        }
        base
    }

    #[test]
    fn loads_local_and_service_scripts_in_sorted_order() {
        let base = build();
        let scripts = load(&base.join("cmd")).unwrap();
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(scripts[0].subdir, None);
        assert_eq!(scripts[1].subdir.as_deref(), Some("api"));
        // Deep scripts keep the top-level service name.
        assert_eq!(scripts[2].subdir.as_deref(), Some("api"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn returns_an_empty_list_when_the_directory_is_missing() {
        let base = build();
        let scripts = load(&base.join("cmd").join("nope")).unwrap();
        assert!(scripts.is_empty());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn select_matches_with_or_without_the_extension() {
        let base = build();
        let scripts = load(&base.join("cmd")).unwrap();
        assert_eq!(select(&scripts, "a").unwrap().name, "a");
        assert_eq!(
            select(&scripts, "b.sh").unwrap().subdir.as_deref(),
            Some("api")
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn select_reports_an_unknown_name_with_the_available_scripts() {
        let base = build();
        let scripts = load(&base.join("cmd")).unwrap();
        let err = select(&scripts, "nope").unwrap_err();
        match err {
            Error::ScriptNotFound { name, available } => {
                assert_eq!(name, "nope");
                assert!(available.contains(&"a".to_string()));
                assert!(available.contains(&"b (docker: api)".to_string()));
            }
            other => panic!("unexpected error: {other:?}"),
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn select_reports_a_name_found_in_two_locations() {
        let base = build();
        std::fs::write(base.join("cmd").join("api").join("a.sh"), "# dup").unwrap();
        let scripts = load(&base.join("cmd")).unwrap();
        let err = select(&scripts, "a").unwrap_err();
        match err {
            Error::AmbiguousScript { name, matches } => {
                assert_eq!(name, "a");
                assert_eq!(matches.len(), 2);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
