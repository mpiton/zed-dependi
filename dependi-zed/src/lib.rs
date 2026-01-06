use zed_extension_api::{self as zed, LanguageServerId, Result};

struct DependiExtension {
    cached_binary_path: Option<String>,
}

impl DependiExtension {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
    ) -> Result<String> {
        // Return cached path if valid
        if let Some(path) = &self.cached_binary_path
            && std::fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
        {
            return Ok(path.clone());
        }

        // Check for local binary first
        let local_binary = "dependi-lsp-1.0.0/dependi-lsp";
        if std::fs::metadata(local_binary).map(|m| m.is_file()).unwrap_or(false) {
            zed::log(&format!("Using local binary: {}", local_binary));
            self.cached_binary_path = Some(local_binary.to_string());
            return Ok(local_binary.to_string());
        }

        zed::log("Local binary not found, downloading from GitHub");

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

            zed::download_file(
                &asset.download_url,
                &version_dir,
                file_type,
            )
            .map_err(|e| format!("Failed to download: {e}"))?;

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
