use std::path::Path;

use bot_config::{BotConfig, ConfigSourcePaths, load_from_paths};
use figment::{
    Figment,
    providers::{Format, Toml},
};

#[test]
fn example_toml_loads_cleanly_and_has_no_unknown_or_invalid_keys() {
    let example_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/bot.toml.example");
    assert!(
        example_path.is_file(),
        "packaging/bot.toml.example must exist"
    );

    // Ensure strict deserialization (no typos/drift) and full safety validation.
    let parsed: BotConfig = Figment::new()
        .merge(Toml::file(&example_path))
        .extract()
        .expect("strict BotConfig parse from packaging/bot.toml.example");
    let loaded = load_from_paths(&ConfigSourcePaths {
        base_file: Some(example_path),
        conf_d_dir: None,
        env_prefix: None,
    })
    .expect("example config passes load + validate");
    assert_eq!(parsed.general.mode, loaded.general.mode);
}
