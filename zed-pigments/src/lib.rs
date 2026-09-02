use std::fs;

use zed_extension_api::{self as zed, Result};

const GITHUB_REPO: &str = "JonathonRP/zed-pigments";
const SERVER_NAME: &str = "pigment-lsp";

struct ZedPigmentsExtension {
    cached_binary_path: Option<String>,
}

enum Status {
    None,
    Downloading,
    Failed(String),
}

fn update_status(id: &zed::LanguageServerId, status: Status) {
    let status = match status {
        Status::None => zed::LanguageServerInstallationStatus::None,
        Status::Downloading => zed::LanguageServerInstallationStatus::Downloading,
        Status::Failed(message) => zed::LanguageServerInstallationStatus::Failed(message),
    };
    zed::set_language_server_installation_status(id, &status);
}

fn binary_name() -> &'static str {
    if zed::current_platform().0 == zed::Os::Windows {
        "pigment-lsp.exe"
    } else {
        SERVER_NAME
    }
}

fn release_asset_name() -> Result<String> {
    let (os, architecture) = zed::current_platform();
    release_asset_name_for(os, architecture)
}

fn release_asset_name_for(os: zed::Os, architecture: zed::Architecture) -> Result<String> {
    let os_name = match os {
        zed::Os::Mac => "darwin",
        zed::Os::Linux => "linux",
        zed::Os::Windows => "windows",
    };
    let architecture_name = match architecture {
        zed::Architecture::Aarch64 => "arm64",
        zed::Architecture::X8664 => "amd64",
        zed::Architecture::X86 => {
            return Err("Zed Pigments does not publish 32-bit binaries".to_owned())
        }
    };
    let extension = if os == zed::Os::Windows {
        "zip"
    } else {
        "tar.gz"
    };
    Ok(format!(
        "pigment-lsp-{os_name}-{architecture_name}.{extension}"
    ))
}

fn installed_binary() -> Option<String> {
    let mut candidates = fs::read_dir(".")
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_str().is_some_and(is_version_directory))
        .filter_map(|entry| {
            let path = entry.path().join(binary_name());
            let modified = path.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(modified, _)| *modified);
    candidates
        .pop()
        .and_then(|(_, path)| path.to_str().map(str::to_owned))
}

fn cleanup_old_versions(current_version_dir: &str) -> Result<()> {
    let entries = fs::read_dir(".")
        .map_err(|error| format!("failed to inspect installed pigment-lsp versions: {error}"))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to inspect an installed version: {error}"))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if is_version_directory(name)
            && name != current_version_dir
            && entry
                .file_type()
                .map_err(|error| format!("failed to inspect {name}: {error}"))?
                .is_dir()
        {
            fs::remove_dir_all(entry.path())
                .map_err(|error| format!("failed to remove obsolete {name}: {error}"))?;
        }
    }
    Ok(())
}

fn is_version_directory(name: &str) -> bool {
    name.strip_prefix("pigment-lsp-")
        .and_then(|version| version.strip_prefix('v').or(Some(version)))
        .and_then(|version| version.bytes().next())
        .is_some_and(|first| first.is_ascii_digit())
}

impl ZedPigmentsExtension {
    fn language_server_binary_path(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        if let Some(path) = worktree.which(binary_name()) {
            return Ok(path);
        }

        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
                update_status(id, Status::None);
                return Ok(path.clone());
            }
        }

        let release = match zed::latest_github_release(
            GITHUB_REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        ) {
            Ok(release) => release,
            Err(error) => {
                if let Some(path) = installed_binary() {
                    update_status(id, Status::None);
                    return Ok(path);
                }
                return Err(format!("failed to query Zed Pigments releases: {error}"));
            }
        };

        let version_dir = format!("pigment-lsp-{}", release.version);
        let binary_path = format!("{version_dir}/{}", binary_name());
        if !fs::metadata(&binary_path).is_ok_and(|metadata| metadata.is_file()) {
            let asset_name = release_asset_name()?;
            let asset = release
                .assets
                .iter()
                .find(|asset| asset.name == asset_name)
                .ok_or_else(|| format!("release {} has no {asset_name} asset", release.version))?;
            let file_type = if zed::current_platform().0 == zed::Os::Windows {
                zed::DownloadedFileType::Zip
            } else {
                zed::DownloadedFileType::GzipTar
            };

            update_status(id, Status::Downloading);
            zed::download_file(&asset.download_url, &version_dir, file_type)
                .map_err(|error| format!("failed to download {asset_name}: {error}"))?;
            if !fs::metadata(&binary_path).is_ok_and(|metadata| metadata.is_file()) {
                return Err(format!(
                    "{asset_name} did not contain {} at its archive root",
                    binary_name()
                ));
            }
        }

        cleanup_old_versions(&version_dir)?;
        update_status(id, Status::None);
        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for ZedPigmentsExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let command = self
            .language_server_binary_path(id, worktree)
            .inspect_err(|error| update_status(id, Status::Failed(error.to_string())))?;
        Ok(zed::Command {
            command,
            args: Vec::new(),
            env: Default::default(),
        })
    }
}

zed::register_extension!(ZedPigmentsExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_assets_match_workflow_names() {
        assert_eq!(
            release_asset_name_for(zed::Os::Linux, zed::Architecture::X8664).unwrap(),
            "pigment-lsp-linux-amd64.tar.gz"
        );
        assert_eq!(
            release_asset_name_for(zed::Os::Linux, zed::Architecture::Aarch64).unwrap(),
            "pigment-lsp-linux-arm64.tar.gz"
        );
        assert_eq!(
            release_asset_name_for(zed::Os::Mac, zed::Architecture::X8664).unwrap(),
            "pigment-lsp-darwin-amd64.tar.gz"
        );
        assert_eq!(
            release_asset_name_for(zed::Os::Windows, zed::Architecture::Aarch64).unwrap(),
            "pigment-lsp-windows-arm64.zip"
        );
        assert!(release_asset_name_for(zed::Os::Windows, zed::Architecture::X86).is_err());
    }
}
