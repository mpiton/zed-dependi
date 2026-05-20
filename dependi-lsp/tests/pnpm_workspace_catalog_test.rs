use dependi_lsp::parsers::Parser;
use dependi_lsp::parsers::npm::NpmParser;
use dependi_lsp::parsers::pnpm_workspace::{
    PnpmWorkspaceParser, read_pnpm_workspace_for_package, resolve_catalog_references,
};

fn dependency_pairs(content: &str) -> Vec<(String, String)> {
    let mut pairs = PnpmWorkspaceParser::new()
        .parse(content)
        .into_iter()
        .map(|dependency| (dependency.name, dependency.version))
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    pairs
}

#[test]
fn default_catalog_entries_accept_inline_comments_and_quoted_versions() {
    let workspace_yaml = r#"
packages: [packages/*]
catalog: # shared dependency versions
  git-dep: "github:example/repo#v1.2.3" # pinned git ref
  react: "^18.3.1" # pinned for React 18 apps
  redux: '^5.0.1'
"#;

    assert_eq!(
        dependency_pairs(workspace_yaml),
        vec![
            (
                "git-dep".to_string(),
                "github:example/repo#v1.2.3".to_string(),
            ),
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
        (
            "packages: [packages/*]\ncatalog: { react: ^18.3.1, redux: ^5.0.1 }\n",
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
fn named_catalog_shapes_determine_discovered_npm_dependencies() {
    // Given a workspace file "pnpm-workspace.yaml" represented as compact YAML "<workspace_yaml>"
    // When Dependi inspects "pnpm-workspace.yaml"
    // Then the discovered npm dependencies are "<expected_dependencies>"
    let cases = [
        (
            "packages: [packages/*]\ncatalogs:\n  react18:\n    react: ^18.2.0\n    react-dom: ^18.2.0\n",
            vec![
                ("react".to_string(), "^18.2.0".to_string()),
                ("react-dom".to_string(), "^18.2.0".to_string()),
            ],
        ),
        (
            "packages: [packages/*]\ncatalogs: { react18: { react: ^18.2.0, react-dom: ^18.2.0 } }\n",
            vec![
                ("react".to_string(), "^18.2.0".to_string()),
                ("react-dom".to_string(), "^18.2.0".to_string()),
            ],
        ),
        (
            "packages: [packages/*]\ncatalogs:\n  react18: { react: ^18.2.0, react-dom: ^18.2.0 }\n",
            vec![
                ("react".to_string(), "^18.2.0".to_string()),
                ("react-dom".to_string(), "^18.2.0".to_string()),
            ],
        ),
        (
            "packages: [packages/*]\ncatalogs:\n  react18: {}\n",
            Vec::new(),
        ),
        ("packages: [packages/*]\ncatalogs: {}\n", Vec::new()),
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

#[test]
fn catalog_shorthand_ignores_named_catalog_entries_without_default_catalog() {
    let workspace_yaml = r#"
packages:
  - packages/*

catalogs:
  react18:
    react: ^18.2.0
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
    assert_eq!(react.version, "catalog:");
}

#[test]
fn react_dom_named_catalog_reference_resolves_through_react18() {
    // Given a workspace file "pnpm-workspace.yaml" containing:
    //   packages:
    //     - packages/*
    //
    //   catalogs:
    //     react18:
    //       react: ^18.2.0
    //       react-dom: ^18.2.0
    // And a package file "packages/example-components/package.json" containing:
    //   {
    //     "name": "@example/components",
    //     "dependencies": {
    //       "react-dom": "catalog:react18"
    //     }
    //   }
    // When Dependi inspects "packages/example-components/package.json"
    // Then the dependency "react-dom" resolves to npm version range "^18.2.0"
    let workspace_yaml = r#"
packages:
  - packages/*

catalogs:
  react18:
    react: ^18.2.0
    react-dom: ^18.2.0
"#;
    let package_json = r#"{
  "name": "@example/components",
  "dependencies": {
    "react-dom": "catalog:react18"
  }
}"#;

    let dependencies =
        resolve_catalog_references(NpmParser::new().parse(package_json), Some(workspace_yaml));

    let react_dom = dependencies
        .iter()
        .find(|dependency| dependency.name == "react-dom")
        .unwrap();
    assert_eq!(react_dom.version, "^18.2.0");
}

#[tokio::test]
async fn package_json_catalog_resolution_reads_nearest_workspace_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace_path = tmp.path().join("pnpm-workspace.yaml");
    let package_dir = tmp.path().join("packages").join("example-app");
    let package_path = package_dir.join("package.json");

    std::fs::create_dir_all(&package_dir).expect("create package dir");
    std::fs::write(
        &workspace_path,
        "packages: [packages/*]\ncatalog: { react: ^18.3.1 }\n",
    )
    .expect("write workspace");
    std::fs::write(
        &package_path,
        r#"{
  "dependencies": {
    "react": "catalog:"
  }
}"#,
    )
    .expect("write package");

    let package_json = std::fs::read_to_string(&package_path).expect("read package");
    let workspace_content = read_pnpm_workspace_for_package(&package_path).await;
    let dependencies = resolve_catalog_references(
        NpmParser::new().parse(&package_json),
        workspace_content.as_deref(),
    );

    let react = dependencies
        .iter()
        .find(|dependency| dependency.name == "react")
        .unwrap();
    assert_eq!(react.version, "^18.3.1");
}
