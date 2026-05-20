use dependi_lsp::parsers::Parser;
use dependi_lsp::parsers::npm::NpmParser;
use dependi_lsp::parsers::pnpm_workspace::{PnpmWorkspaceParser, resolve_catalog_references};

fn dependency_pairs(content: &str) -> Vec<(String, String)> {
    PnpmWorkspaceParser::new()
        .parse(content)
        .into_iter()
        .map(|dependency| (dependency.name, dependency.version))
        .collect()
}

#[test]
fn default_catalog_entries_accept_inline_comments_and_quoted_versions() {
    let workspace_yaml = r#"
packages: [packages/*]
catalog: # shared dependency versions
  react: "^18.3.1" # pinned for React 18 apps
  redux: '^5.0.1'
"#;

    assert_eq!(
        dependency_pairs(workspace_yaml),
        vec![
            ("react".to_string(), "^18.3.1".to_string()),
            ("redux".to_string(), "^5.0.1".to_string()),
        ]
    );
}

#[test]
fn default_catalog_shapes_determine_discovered_npm_dependencies() {
    // Given a workspace file "pnpm-workspace.yaml" represented as compact YAML "<workspace_yaml>"
    // When Dependi inspects "pnpm-workspace.yaml"
    // Then the discovered npm dependencies are "<expected_dependencies>"
    let cases = [
        (
            "packages: [packages/*]\ncatalog:\n  react: ^18.3.1\n  redux: ^5.0.1\n",
            vec![
                ("react".to_string(), "^18.3.1".to_string()),
                ("redux".to_string(), "^5.0.1".to_string()),
            ],
        ),
        ("packages: [packages/*]\ncatalog: {}\n", Vec::new()),
        ("packages: [packages/*]\n", Vec::new()),
    ];

    for (workspace_yaml, expected_dependencies) in cases {
        assert_eq!(dependency_pairs(workspace_yaml), expected_dependencies);
    }
}

#[test]
fn react_catalog_shorthand_resolves_through_the_default_catalog() {
    // Given a workspace file "pnpm-workspace.yaml" containing:
    //   packages:
    //     - packages/*
    //
    //   catalog:
    //     react: ^18.3.1
    //     redux: ^5.0.1
    // And a package file "packages/example-app/package.json" containing:
    //   {
    //     "name": "@example/app",
    //     "dependencies": {
    //       "react": "catalog:"
    //     }
    //   }
    // When Dependi inspects "packages/example-app/package.json"
    // Then the dependency "react" resolves to npm version range "^18.3.1"
    let workspace_yaml = r#"
packages:
  - packages/*

catalog:
  react: ^18.3.1
  redux: ^5.0.1
"#;
    let package_json = r#"{
  "name": "@example/app",
  "dependencies": {
    "react": "catalog:"
  }
}"#;

    let dependencies =
        resolve_catalog_references(NpmParser::new().parse(package_json), Some(workspace_yaml));

    let react = dependencies
        .iter()
        .find(|dependency| dependency.name == "react")
        .unwrap();
    assert_eq!(react.version, "^18.3.1");
}
