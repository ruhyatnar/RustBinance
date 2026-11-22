//! Layered configuration loader (`defaults -> bot.toml -> conf.d/*.toml -> BOT__* env`).

use std::fs;
use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Serialized, Toml};
use figment::value::{Dict, Map, Value};
use figment::{Figment, Metadata, Profile, Provider};

use crate::{BotConfig, ConfigError};

/// Filesystem paths and environment prefix defining the configuration layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSourcePaths {
    /// Optional primary TOML file (default: `/etc/binance-bot/bot.toml`).
    pub base_file: Option<PathBuf>,
    /// Optional drop-in directory for lexicographically sorted `*.toml` overrides.
    pub conf_d_dir: Option<PathBuf>,
    /// Optional environment variable prefix (default: `"BOT__"`).
    pub env_prefix: Option<String>,
}

impl ConfigSourcePaths {
    /// Standard production/systemd paths specified in Blueprint §9.
    #[must_use]
    pub fn system_default() -> Self {
        Self {
            base_file: Some(PathBuf::from("/etc/binance-bot/bot.toml")),
            conf_d_dir: Some(PathBuf::from("/etc/binance-bot/conf.d")),
            env_prefix: Some("BOT__".to_owned()),
        }
    }
}

struct EnvPairsProvider<'a> {
    pairs: &'a [(&'a str, &'a str)],
}

impl Provider for EnvPairsProvider<'_> {
    fn metadata(&self) -> Metadata {
        Metadata::named("in-memory env overrides")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut root = Dict::new();
        for &(raw_key, raw_val) in self.pairs {
            let parts: Vec<String> = raw_key
                .split("__")
                .filter(|seg| !seg.is_empty())
                .map(str::to_ascii_lowercase)
                .collect();
            if parts.is_empty() {
                continue;
            }
            let parsed_value = raw_val
                .parse::<Value>()
                .unwrap_or_else(|_| Value::from(raw_val.to_owned()));
            insert_nested(&mut root, &parts, parsed_value);
        }
        let mut map = Map::new();
        map.insert(Profile::Default, root);
        Ok(map)
    }
}

fn insert_nested(dict: &mut Dict, path: &[String], value: Value) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    if rest.is_empty() {
        dict.insert(first.clone(), value);
        return;
    }
    let entry = dict
        .entry(first.clone())
        .or_insert_with(|| Value::from(Dict::new()));
    if let Value::Dict(_, child) = entry {
        insert_nested(child, rest, value);
    } else {
        let mut child = Dict::new();
        insert_nested(&mut child, rest, value);
        *entry = Value::from(child);
    }
}

fn sorted_conf_d_files(dir: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    let entries = fs::read_dir(dir).map_err(|err| ConfigError::Io {
        path: dir.to_path_buf(),
        message: err.to_string(),
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| ConfigError::Io {
            path: dir.to_path_buf(),
            message: err.to_string(),
        })?;
        let path = entry.path();
        let is_toml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
        if is_toml && path.is_file() {
            files.push(path);
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

fn build_figment(paths: &ConfigSourcePaths) -> Result<Figment, ConfigError> {
    let mut figment = Figment::from(Serialized::defaults(BotConfig::default()));
    if let Some(base) = &paths.base_file {
        if base.exists() {
            figment = figment.merge(Toml::file(base));
        }
    }
    if let Some(conf_d) = &paths.conf_d_dir {
        if conf_d.exists() {
            for file in sorted_conf_d_files(conf_d)? {
                figment = figment.merge(Toml::file(file));
            }
        }
    }
    if let Some(prefix) = &paths.env_prefix {
        figment = figment.merge(Env::prefixed(prefix).split("__"));
    }
    Ok(figment)
}

/// Loads [`BotConfig`] from the standard system paths (`/etc/binance-bot/bot.toml`, `conf.d/*.toml`, `BOT__*`).
///
/// # Errors
/// Returns [`ConfigError`] if any file cannot be read, contains unknown fields or malformed TOML,
/// or fails validation.
pub fn load_config() -> Result<BotConfig, ConfigError> {
    load_from_paths(&ConfigSourcePaths::system_default())
}

/// Loads [`BotConfig`] from explicit filesystem layers and optional environment prefix.
///
/// # Errors
/// Returns [`ConfigError`] on I/O, parse, unknown-field, or validation errors.
pub fn load_from_paths(paths: &ConfigSourcePaths) -> Result<BotConfig, ConfigError> {
    let figment = build_figment(paths)?;
    let cfg = figment
        .extract::<BotConfig>()
        .map_err(|err| ConfigError::Load(err.to_string()))?;
    cfg.validate()?;
    Ok(cfg)
}

/// Loads [`BotConfig`] from explicit filesystem layers plus in-memory `SECTION__KEY` override pairs.
///
/// # Errors
/// Returns [`ConfigError`] on I/O, parse, unknown-field, or validation errors.
pub fn load_from_paths_with_env_pairs(
    paths: &ConfigSourcePaths,
    env_pairs: &[(&str, &str)],
) -> Result<BotConfig, ConfigError> {
    let figment = build_figment(paths)?.merge(EnvPairsProvider { pairs: env_pairs });
    let cfg = figment
        .extract::<BotConfig>()
        .map_err(|err| ConfigError::Load(err.to_string()))?;
    cfg.validate()?;
    Ok(cfg)
}

/// Loads [`BotConfig`] by layering a TOML string on top of code defaults.
///
/// # Errors
/// Returns [`ConfigError`] on malformed TOML, unknown fields, or validation errors.
pub fn load_from_toml_str(toml_str: &str) -> Result<BotConfig, ConfigError> {
    let cfg = Figment::from(Serialized::defaults(BotConfig::default()))
        .merge(Toml::string(toml_str))
        .extract::<BotConfig>()
        .map_err(|err| ConfigError::Load(err.to_string()))?;
    cfg.validate()?;
    Ok(cfg)
}
