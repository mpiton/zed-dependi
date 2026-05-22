use std::collections::BTreeSet;

use toml::Value;

const DEPENDI_LSP_MANIFEST: &str = include_str!("../Cargo.toml");
const EXPECTED_TOKIO_FEATURES: [&str; 4] = ["fs", "io-util", "macros", "rt-multi-thread"];

fn tokio_features(manifest: &str) -> BTreeSet<String> {
    let manifest = toml::from_str::<Value>(manifest).expect("dependi-lsp Cargo.toml is valid TOML");
    manifest["dependencies"]["tokio"]["features"]
        .as_array()
        .expect("tokio dependency declares explicit features")
        .iter()
        .map(|feature| {
            feature
                .as_str()
                .expect("tokio features are strings")
                .to_string()
        })
        .collect()
}

fn expected_tokio_features() -> BTreeSet<String> {
    EXPECTED_TOKIO_FEATURES
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[test]
fn dependi_lsp_uses_the_trimmed_tokio_feature_set() {
    // Given the `dependi-lsp/Cargo.toml` dependency line for `tokio` is:
    //   tokio = { version = "1.52.3", features = ["rt-multi-thread", "macros", "io-util", "fs"] }
    // When the Tokio dependency features are inspected
    // Then the direct Tokio feature set is exactly "fs, io-util, macros, rt-multi-thread"
    // And the direct Tokio feature set does not include "full"
    let features = tokio_features(DEPENDI_LSP_MANIFEST);

    assert_eq!(features, expected_tokio_features());
    assert!(!features.contains("full"));
}
