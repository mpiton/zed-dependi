# Support Maven / pom.xml (Java) — Design

**Issue** : [#223](https://github.com/mpiton/zed-dependi/issues/223) — feat: Add support for Java's Maven (pom.xml)
**Date** : 2026-04-17
**Scope** : MVP réaliste (scope B)
**API strategy** : Hybride maven-metadata.xml + POM best-effort (stratégie C)
**Registres alternatifs** : URL base configurable, sans auth (option B)

## Objectif

Ajouter le support de l'écosystème Java/Maven à Zed Dependi : parsing de `pom.xml`, fetch de versions via Maven Central, détection de vulnérabilités via OSV.dev, et parité fonctionnelle avec les 8 écosystèmes existants (Rust, JS, Python, Go, PHP, Dart, C#, Ruby).

## Non-goals (hors MVP)

- **Résolution de parent POM** (téléchargement récursif) — détection + NOOP seulement
- **BOM / dependencyManagement import** (téléchargement de POM de BOM) — parsing local seulement
- **Auth pour registres privés** (Nexus/Artifactory avec Bearer token) — URL configurable seulement
- **Multi-module projects** (parcours récursif des sous-modules) — chaque pom.xml traité indépendamment
- **Plugin dependencies** (dépendances dans `<build><plugins>`) — ignorées
- **Lockfile** : Maven n'a pas de lockfile standard (contrairement à Cargo/npm)

## Architecture

### Nouveaux fichiers

- `dependi-lsp/src/parsers/maven.rs` — `MavenParser` implémentant `Parser`
- `dependi-lsp/src/registries/maven_central.rs` — `MavenCentralRegistry` implémentant `Registry`

### Fichiers modifiés (~15 points d'intégration)

| Fichier | Points | Description |
|---|---|---|
| `file_types.rs` | 6 | Enum `Maven`, `detect()`, `to_ecosystem()`, `fmt_registry_package_url()`, `registry_name()`, `cache_key()` |
| `vulnerabilities/mod.rs` | 2 | `Ecosystem::Maven`, `as_osv_str()` |
| `backend.rs` | 6 | Imports, `ProcessingContext` fields, `parse_document()`, `with_http_client()`, `create_processing_context()`, dispatcher dans `process_document()` et `get_version_info()` public |
| `parsers/mod.rs` | 1 | `pub mod maven;` |
| `registries/mod.rs` | 1 | `pub mod maven_central;` |
| `config.rs` | 1 | `MavenRegistryConfig { url }` + ajout à `RegistriesConfig` |
| `providers/code_actions.rs` | 1 | Fallback dans `compare_update_type()` pour versions non-semver (SNAPSHOT) |
| `Cargo.toml` | 1 | `quick-xml` dépendance |

### Flux de données

```
pom.xml détecté (file_types.rs) → MavenParser.parse() → Vec<Dependency>
    ↓
backend.rs dispatch → MavenCentralRegistry.get_version_info()
    ↓ requête 1 : maven-metadata.xml → versions[]
    ↓ requête 2 (best-effort parallèle) : POM du dernier release → license + description
    ↓
VersionInfo (cached dans HybridCache) → Providers (inlay hints, diagnostics, code actions, links, completion)
    ↓
OSV.dev batch query (ecosystem="Maven") → vulnerabilities intégrées
```

## `MavenParser` (parsers/maven.rs)

### Librairie

`quick-xml` 0.38+ en streaming mode (`Reader::read_event()`) pour :
- Parcourir les tags avec peu d'allocations
- Capturer positions via `reader.buffer_position()` (byte-offset)
- Convertir byte-offset → (line, column) avec helper type `npm.rs:compute_line_offsets`

### Algorithme : 2 passes

**Pass 1** — `extract_properties(content) → HashMap<String, String>` :
- Parcourt tags `<properties>...<key>value</key>...</properties>`
- Populate map : `spring.version → "6.1.0"`
- Inclut built-ins détectés depuis `<project>` : `project.version`, `project.groupId`, `project.artifactId`

**Pass 2** — extraction des dépendances :
- Tracker state-machine : `InProject | InDependencies | InDependencyManagement | Other`
- Accepter `<dependency>` dans `<dependencies>` ET `<dependencyManagement><dependencies>`
- Pour chaque dépendance :
  - Capturer `groupId`, `artifactId`, `version`, `scope`, `optional`
  - Substituer `${foo}` via properties map ; si non résolu → garder littéral
  - Calculer positions byte-offset du contenu `<version>` → (line, col)

### Mapping `Dependency`

| Champ | Source |
|---|---|
| `name` | `"{groupId}:{artifactId}"` |
| `version` | `<version>` substitué (ou littéral `${...}` si non résolu) |
| `line/name_start/name_end/version_start/version_end` | Positions XML calculées |
| `dev` | `scope == "test" \|\| scope == "provided"` |
| `optional` | `<optional>true</optional>` |
| `registry` | `None` |
| `resolved_version` | `None` |

### Edge cases

1. **Snapshots** : `1.0-SNAPSHOT` passé tel quel (registry le classifiera)
2. **Ranges Maven** `[1.0,2.0)` : passés tels quels, pas de pseudo-semver
3. **Property non résolue** : `${foo}` littéral préservé ; le backend gérera l'erreur du fetch
4. **`<parent>` détecté** : log debug, aucune action (MVP)
5. **dependencyManagement sans version** : skip
6. **Classifier/type** : parser mais ignoré (OSV ne les utilise pas)
7. **XML invalide** : retour `vec![]` (pattern existants)

### Tests unitaires (inline)

- `test_parse_simple_dependency`
- `test_parse_with_properties`
- `test_parse_scope_test_marked_as_dev`
- `test_parse_scope_provided_marked_as_dev`
- `test_parse_optional`
- `test_parse_snapshot_version`
- `test_parse_dependency_management`
- `test_parse_unresolved_property_preserved`
- `test_parse_invalid_xml_returns_empty`
- `test_parse_position_tracking`
- `test_parse_nested_dependency_in_plugin_ignored`
- `test_parse_project_version_builtin`

## `MavenCentralRegistry` (registries/maven_central.rs)

### Structure

```rust
pub struct MavenCentralRegistry {
    client: Arc<Client>,
    base_url: String,  // "https://repo1.maven.org/maven2"
}

impl MavenCentralRegistry {
    pub fn with_client(client: Arc<Client>) -> Self;
    pub fn with_client_and_config(client: Arc<Client>, config: MavenRegistryConfig) -> Self;
}

impl Registry for MavenCentralRegistry {
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo>;
    fn http_client(&self) -> Arc<Client> { self.client.clone() }
}
```

### Algorithme `get_version_info(package_name)`

1. **Parse** `"groupId:artifactId"` → split sur `:`. Erreur si mal formé.
2. **Construire URL metadata** :
   `{base_url}/{groupId.replace('.', '/')}/{artifactId}/maven-metadata.xml`
3. **Requête 1 (GET metadata.xml)** → parser avec quick-xml :
   - `<versioning><release>` → `latest`
   - `<versioning><versions><version>*</version></versions>` → liste complète
   - Détecter SNAPSHOTs/alpha/beta/rc → `latest_prerelease`
4. **Requête 2 (best-effort séquentielle)** : GET POM du dernier release
   - URL : `{base_url}/{gpath}/{artifactId}/{latest}/{artifactId}-{latest}.pom`
   - Parser avec quick-xml → extraire :
     - `<description>` → `description`
     - `<url>` → `homepage`
     - `<scm><url>` → `repository`
     - `<licenses><license><name>` (joined `, ` si plusieurs) → `license`
   - Échec → log warning, continuer avec VersionInfo partiel
5. **Retour** `VersionInfo` :
   - `latest`, `latest_prerelease`, `versions` triées descendant (comparator Maven simplifié)
   - `description`, `homepage`, `repository`, `license` depuis POM (ou None)
   - `deprecated: false`, `yanked: false`, `yanked_versions: vec![]`
   - `release_dates: HashMap::new()` (metadata.xml n'expose pas de dates par version)

### Erreurs

- HTTP 404 sur metadata → `anyhow::bail!("Package not found: {package_name}")`
- XML metadata invalide → `anyhow::bail!("Invalid Maven metadata XML")`
- POM fetch/parse fail → log warning (non bloquant)
- `package_name` sans `:` → `anyhow::bail!("Invalid Maven coordinate (expected groupId:artifactId)")`

### Version comparator

Helper `compare_maven_versions(a, b) -> Ordering` :
- Parse simplifié `major.minor.patch[-qualifier]`
- Snapshots toujours < release équivalent (`1.0-SNAPSHOT` < `1.0`)
- Fallback lexical si parsing échoue

### Config

```rust
// config.rs
pub struct MavenRegistryConfig {
    #[serde(default = "default_maven_url")]
    pub url: String,  // "https://repo1.maven.org/maven2"
}

fn default_maven_url() -> String {
    "https://repo1.maven.org/maven2".to_string()
}

pub struct RegistriesConfig {
    pub npm: NpmRegistryConfig,
    pub cargo: CargoRegistryConfig,
    pub maven: MavenRegistryConfig,  // nouveau
}
```

### Tests unitaires (inline)

- `test_parse_metadata_xml_basic`
- `test_parse_pom_extracts_license_and_description`
- `test_parse_pom_missing_license_returns_none`
- `test_url_construction_group_to_path`
- `test_invalid_package_name_errors`
- `test_snapshot_classified_as_prerelease`
- `test_compare_maven_versions`

## Intégration backend

### `file_types.rs`

```rust
pub enum FileType { ..., Maven }

// detect()
else if path.ends_with("pom.xml") { Some(FileType::Maven) }

// to_ecosystem()
FileType::Maven => Ecosystem::Maven

// fmt_registry_package_url()
FileType::Maven => {
    let url_path = name.replace(':', "/");
    write!(f, "https://mvnrepository.com/artifact/{url_path}")
}

// registry_name()
FileType::Maven => "Maven Central"

// fmt_cache_key()
FileType::Maven => write!(f, "maven:{package_name}")
```

### `vulnerabilities/mod.rs`

```rust
pub enum Ecosystem { ..., Maven }

// as_osv_str()
Ecosystem::Maven => "Maven"
```

### `backend.rs`

- Import `MavenParser`, `MavenCentralRegistry`
- Nouveaux champs `maven_parser: Arc<MavenParser>`, `maven_central: Arc<MavenCentralRegistry>` dans `Backend` et `ProcessingContext`
- Dispatcher dans `parse_document()` : `Some(FileType::Maven) => self.maven_parser.parse(content)`
- Dispatcher dans `process_document()` : `FileType::Maven => maven_central.get_version_info(&name).await`
- Initialisation dans `with_http_client()` (lire `config.registries.maven`)
- Clonage Arc dans `create_processing_context()`
- Dispatcher dans `get_version_info()` public

**Pas de lockfile resolution** — aucune section ajoutée dans `process_document()` pour la partie lockfile.

### UX spécifiques

1. **Inlay hints** — fonctionnent via le flux générique
2. **Diagnostics** — fonctionnent via le flux générique
3. **Code actions** — adaptation mineure dans `code_actions.rs` : `compare_update_type()` doit fallback à `VersionUpdateType::Patch` quand `semver::Version::parse` échoue (cas de `1.0-SNAPSHOT`)
4. **Document links** — fonctionnent via `fmt_registry_package_url()`
5. **Completion** — fonctionne (agnostique)
6. **Properties non résolues** — `${foo}` littéral conservé ; le fetch échouera et produira un diagnostic standard "package not found"
7. **Parent POM** — pas de UX spéciale MVP

## Tests

### Unitaires (inline `#[cfg(test)] mod tests`)

- `parsers/maven.rs` : ~12 tests
- `registries/maven_central.rs` : ~7 tests
- `file_types.rs` : ajouts dans tests existants
- `vulnerabilities/mod.rs` : ajouts dans tests existants
- `providers/code_actions.rs` : ajout d'un test pour la fallback SNAPSHOT

### Fixtures

String inline dans les tests (pattern du projet) :
- pom.xml minimal
- pom.xml avec properties
- pom.xml avec scope test/provided
- pom.xml avec dependencyManagement
- maven-metadata.xml d'un artefact réel (ex: `org.slf4j:slf4j-api`)
- POM file avec licenses multiples

### Intégration (`tests/integration_test.rs`)

Ajouter un test vérifiant le flow : détection `pom.xml` → parse → cache lookup → retour.

### Validation CI

Tous les checks existants doivent passer :
- `cargo build --release --package dependi-lsp`
- `cargo test --lib`
- `cargo test --test integration_test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --all -- --check`

## CHANGELOG.md

Entrée à ajouter dans `[Unreleased]` / `Added` :

```markdown
### Added
- Support for Java/Maven projects (pom.xml):
  - Parse direct dependencies with `${properties}` substitution
  - Scope awareness (test/provided marked as dev dependencies)
  - Fetch versions and metadata from Maven Central (maven-metadata.xml + POM)
  - Vulnerability scanning via OSV.dev (Maven ecosystem)
  - Support for alternative Maven repositories via configurable base URL
```

## Estimation

~2-3 jours de développement (conforme à l'estimation "Medium" de l'issue).

## Décisions documentées

| Décision | Choix | Raison |
|---|---|---|
| Scope MVP | B (réaliste) | Couvre ~80% des cas réels sans complexité parent/BOM |
| Stratégie API | C (hybride) | Parité UX (license/description) vs autres écosystèmes |
| Registres alternatifs | B (URL configurable sans auth) | Couvre mirrors publics Nexus, évite doublement complexité |
| Librairie XML | quick-xml | Perf, positions, streaming, maintenance active |
| Format `name` | `groupId:artifactId` | Convention OSV officielle |
| Détection `pom.xml` | Nom seul | Pas d'ambiguïté (aucun autre outil n'utilise ce nom) |
| Position tracking | Byte-offset du contenu `<version>` | Inlay hints après la version, pattern npm.rs |
| Lockfile | Non supporté | Maven n'a pas de lockfile standard |
