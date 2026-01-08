use sha2::{Digest, Sha256};
use zed_extension_api::{
    self as zed, LanguageServerId, Result,
    http_client::{HttpMethod, HttpRequest, RedirectPolicy},
};

struct DependiExtension {
    cached_binary_path: Option<String>,
}

fn fetch_checksum(release_version: &str, asset_name: &str) -> Result<Option<String>> {
    let checksum_url = format!(
        "https://github.com/mpiton/zed-dependi/releases/download/{}/{}.binary.sha256",
        release_version, asset_name
    );

    let request = HttpRequest::builder()
        .method(HttpMethod::Get)
        .url(&checksum_url)
        .redirect_policy(RedirectPolicy::FollowAll)
        .build()
        .map_err(|e| format!("Failed to build checksum request: {e}"))?;

    match request.fetch() {
        Ok(response) => {
            let body = String::from_utf8(response.body)
                .map_err(|e| format!("Invalid UTF-8 in checksum file: {e}"))?;
            let checksum = body
                .split_whitespace()
                .next()
                .ok_or("Empty checksum file")?
                .to_lowercase();
            Ok(Some(checksum))
        }
        Err(e) if e.contains("404") || e.contains("Not Found") => Ok(None),
        Err(e) => Err(format!("Failed to fetch checksum: {e}")),
    }
}

fn compute_file_sha256(path: &str) -> Result<String> {
    let contents =
        std::fs::read(path).map_err(|e| format!("Failed to read file for checksum: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&contents);
    let hash = hasher.finalize();
    Ok(hex::encode(hash))
}

fn verify_checksum(binary_path: &str, expected: &str) -> Result<()> {
    let actual = compute_file_sha256(binary_path)?;
    if actual != expected {
        return Err(format!(
            "Checksum verification failed!\nExpected: {}\nActual: {}\n\nThe downloaded binary may have been tampered with.",
            expected, actual
        ));
    }
    Ok(())
}

impl DependiExtension {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
    ) -> Result<String> {
        // Return cached path if valid
        if let Some(path) = &self.cached_binary_path
            && std::fs::metadata(path)
                .map(|m| m.is_file())
                .unwrap_or(false)
        {
            return Ok(path.clone());
        }

        // Download from GitHub releases
        let (platform, arch) = zed::current_platform();
        let binary_name = match platform {
            zed::Os::Mac | zed::Os::Linux => "dependi-lsp",
            zed::Os::Windows => "dependi-lsp.exe",
        };

        let target = format!(
            "{}-{}",
            match arch {
                zed::Architecture::Aarch64 => "aarch64",
                zed::Architecture::X8664 => "x86_64",
                zed::Architecture::X86 => "x86",
            },
            match platform {
                zed::Os::Mac => "apple-darwin",
                zed::Os::Linux => "unknown-linux-gnu",
                zed::Os::Windows => "pc-windows-msvc",
            }
        );

        let (asset_name, file_type) = match platform {
            zed::Os::Windows => (
                format!("dependi-lsp-{}.zip", target),
                zed::DownloadedFileType::Zip,
            ),
            _ => (
                format!("dependi-lsp-{}.tar.gz", target),
                zed::DownloadedFileType::GzipTar,
            ),
        };

        let release = zed::latest_github_release(
            "mpiton/zed-dependi",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("No asset found matching {asset_name}"))?;

        let version_dir = format!("dependi-lsp-{}", release.version);
        let binary_path = format!("{version_dir}/{binary_name}");

        if !std::fs::metadata(&binary_path)
            .map(|m| m.is_file())
            .unwrap_or(false)
        {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(&asset.download_url, &version_dir, file_type)
                .map_err(|e| format!("Failed to download: {e}"))?;

            if let Some(expected_checksum) = fetch_checksum(&release.version, &asset_name)? {
                verify_checksum(&binary_path, &expected_checksum)?;
            }

            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::None,
            );
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for DependiExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        _worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let binary_path = self.language_server_binary_path(language_server_id)?;

        Ok(zed::Command {
            command: binary_path,
            args: vec![],
            env: Default::default(),
        })
    }
}

zed::register_extension!(DependiExtension);
