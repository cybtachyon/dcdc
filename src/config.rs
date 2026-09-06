//! Loads and saves `dcdc.toml`, dcdc's user configuration.
//!
//! The project-level file, in the project root, wins over the
//! user-level file in the dcdc home directory. Both hold the same
//! shape: per-plugin settings keyed under a `plugin` table.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// File name of the configuration, in the project root or dcdc home.
pub const FILE_NAME: &str = "dcdc.toml";

/// Per-plugin settings from `dcdc.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginSettings {
    /// Default container for the plugin's sub-commands.
    ///
    /// `local` means the host.
    pub container: String,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            container: "local".to_string(),
        }
    }
}

/// A parsed `dcdc.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Plugin name to settings.
    #[serde(rename = "plugin")]
    pub plugins: BTreeMap<String, PluginSettings>,
}

/// A loaded configuration together with the file it came from,
/// or would be written to, if saved.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// Path of the file, existing or not.
    pub path: PathBuf,
    pub config: Config,
}

/// Returns the dcdc home directory: `$DCDC_HOME` when set, else the
/// user's home directory.
pub fn home_dir() -> Result<PathBuf> {
    std::env::var_os("DCDC_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or(Error::NoHome)
}

/// Locates and loads the configuration that applies to `cwd`.
///
/// A project-level file, in the nearest project root, wins over the
/// user-level file. When neither exists, an empty config is returned
/// alongside the path it would be written to.
pub fn load_for(cwd: &Path) -> Result<Loaded> {
    let root = crate::root::find(cwd).ok();
    let base = match &root {
        Some(root) => root.clone(),
        None => home_dir()?.join(".dcdc"),
    };
    let path = base.join(FILE_NAME);
    let config = if path.is_file() {
        parse(&path)?
    } else {
        Config::default()
    };
    Ok(Loaded { path, config })
}

/// Saves the default container of one plugin, merging into any
/// existing file so its other content and comments survive.
pub fn save_plugin(path: &Path, name: &str, settings: &PluginSettings) -> Result<()> {
    let mut doc = match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| Error::BadConfig {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let table = doc
        .entry("plugin")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let table = table.as_table_mut().ok_or_else(|| Error::BadConfig {
        path: path.to_path_buf(),
        message: "the plugin table has the wrong shape".to_string(),
    })?;
    let sub = table
        .entry(name)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let sub = sub.as_table_mut().ok_or_else(|| Error::BadConfig {
        path: path.to_path_buf(),
        message: format!("the [{plugin}] table has the wrong shape", plugin = name),
    })?;
    sub.insert("container", toml_edit::value(settings.container.clone()));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

/// Parses `path` into a `Config`.
fn parse(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)?;
    let cfg: Config = toml::from_str(&text).map_err(|e| Error::BadConfig {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    /// Builds a `dcdc.toml` at `dir` and returns the path.
    ///
    /// The caller removes the directory.
    fn config_file(dir: &Path, contents: &str) -> PathBuf {
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn loads_plugin_tables_from_a_file() {
        let base = testutil::temp_subdir("config-load");
        let path = config_file(
            &base,
            "# comment to keep\n\n[plugin.shell]\ncontainer = \"api\"\n\n[plugin.other]\ncontainer = \"local\"\n",
        );
        let loaded = parse(&path).unwrap();
        assert_eq!(loaded.plugins["shell"].container, "api");
        assert_eq!(loaded.plugins["other"].container, "local");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_plugin_merges_into_an_existing_file() {
        let base = testutil::temp_subdir("config-save");
        let path = config_file(&base, "# a comment\n\n[plugin.keep]\ncontainer = \"x\"\n");
        save_plugin(&path, "shell", &PluginSettings::default()).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# a comment"), "comment lost: {text}");
        let loaded = parse(&path).unwrap();
        assert_eq!(loaded.plugins["keep"].container, "x");
        assert_eq!(loaded.plugins["shell"].container, "local");

        // Saving again updates in place instead of appending.
        save_plugin(
            &path,
            "shell",
            &PluginSettings {
                container: "web".into(),
            },
        )
        .unwrap();
        let loaded = parse(&path).unwrap();
        assert_eq!(loaded.plugins["shell"].container, "web");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn load_for_prefers_the_project_level_file() {
        let base = testutil::temp_subdir("config-pref");
        // Project root with its own file; the home-level file must
        // lose.
        let proj = base.join("proj");
        std::fs::create_dir_all(proj.join(".dcdc")).unwrap();
        config_file(&proj, "[plugin.shell]\ncontainer = \"proj-c\"\n");
        let home = base.join("home");
        // The home-level file lives under <home>/.dcdc.
        std::fs::create_dir_all(home.join(".dcdc")).unwrap();
        config_file(
            &home.join(".dcdc"),
            "[plugin.shell]\ncontainer = \"home-c\"\n",
        );

        // Edition 2024 makes set_var unsafe; this is the only test
        // that touches this variable.
        unsafe { std::env::set_var("DCDC_HOME", &home) };
        let cwd = base.join("proj").join("deep");
        std::fs::create_dir_all(&cwd).unwrap();
        let loaded = load_for(&cwd).unwrap();
        assert_eq!(loaded.config.plugins["shell"].container, "proj-c");
        assert_eq!(loaded.path, proj.join(FILE_NAME));

        // Outside any project the home-level file applies.
        let other = base.join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        let loaded = load_for(&other).unwrap();
        assert_eq!(loaded.config.plugins["shell"].container, "home-c");
        std::fs::remove_dir_all(base).unwrap();
    }
}
