use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BinaryConfig {
    pub path: String,
    #[serde(default)]
    pub main: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceMapping {
    pub src: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NsisConfig {
    #[serde(rename = "installer-mode")]
    pub installer_mode: String,
    #[serde(rename = "appdata-paths", default)]
    pub appdata_paths: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(rename = "display-name", default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub compression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackagerMetadata {
    #[serde(rename = "product-name")]
    pub product_name: String,
    pub identifier: String,
    pub description: Option<String>,
    pub authors: Option<Vec<String>>,
    pub publisher: Option<String>,
    pub homepage: Option<String>,
    #[serde(rename = "license-file")]
    pub license_file: Option<String>,
    pub icons: Option<Vec<String>>,
    pub formats: Vec<String>,
    pub binaries: Vec<BinaryConfig>,
    #[serde(default)]
    pub resources: Vec<ResourceMapping>,
    #[serde(rename = "before-packaging-command")]
    pub before_packaging_command: Option<String>,
    pub nsis: Option<NsisConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootPackageMetadata {
    pub package: Option<PackageSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSection {
    pub metadata: Option<MetadataSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataSection {
    pub packager: Option<PackagerMetadata>,
}

pub fn parse_packager_config(toml_str: &str) -> Result<PackagerMetadata> {
    // Attempt parsing as direct [package.metadata.packager] or nested root
    if let Ok(root) = toml::from_str::<RootPackageMetadata>(toml_str) {
        if let Some(pkg) = root.package {
            if let Some(meta) = pkg.metadata {
                if let Some(packager) = meta.packager {
                    return Ok(packager);
                }
            }
        }
    }

    #[derive(Deserialize)]
    struct DirectWrapper {
        #[serde(rename = "package.metadata.packager")]
        packager: Option<PackagerMetadata>,
    }

    if let Ok(wrapper) = toml::from_str::<DirectWrapper>(toml_str) {
        if let Some(p) = wrapper.packager {
            return Ok(p);
        }
    }

    // Try parsing as raw PackagerMetadata
    let direct: PackagerMetadata = toml::from_str(toml_str)?;
    Ok(direct)
}

pub fn validate_packaging_requirements(config: &PackagerMetadata) -> Result<()> {
    // 1. Verify before-packaging-command is absent to protect pre-applied signatures
    if let Some(ref cmd) = config.before_packaging_command {
        bail!(
            "Forbidden before-packaging-command '{}': cargo-packager must not rebuild binaries, or Azure signatures will be wiped",
            cmd
        );
    }

    // 2. Verify all three Windows binaries are declared
    let main_binaries: Vec<_> = config.binaries.iter().filter(|b| b.main).collect();
    if main_binaries.len() != 1 {
        bail!("Expected exactly 1 main binary, found {}", main_binaries.len());
    }

    let has_desktop = config.binaries.iter().any(|b| b.path.contains("pandamux.exe") || b.path.ends_with("pandamux"));
    let has_server = config.binaries.iter().any(|b| b.path.contains("pandamux-server.exe") || b.path.ends_with("pandamux-server"));
    let has_cli = config.binaries.iter().any(|b| b.path.contains("pandamux-cli.exe") || b.path.ends_with("pandamux-cli"));

    if !has_desktop {
        bail!("Missing desktop binary 'pandamux.exe' in packager binaries configuration");
    }
    if !has_server {
        bail!("Missing server binary 'pandamux-server.exe' in packager binaries configuration");
    }
    if !has_cli {
        bail!("Missing CLI binary 'pandamux-cli.exe' in packager binaries configuration");
    }

    // 3. Verify Linux node binaries are present in bundled resources
    let has_linux_x86_server = config.resources.iter().any(|r| {
        r.target.contains("x86_64-unknown-linux-musl") && r.target.contains("pandamux-server")
    });
    let has_linux_x86_cli = config.resources.iter().any(|r| {
        r.target.contains("x86_64-unknown-linux-musl") && r.target.contains("pandamux-cli")
    });
    let has_linux_arm_server = config.resources.iter().any(|r| {
        r.target.contains("aarch64-unknown-linux-musl") && r.target.contains("pandamux-server")
    });
    let has_linux_arm_cli = config.resources.iter().any(|r| {
        r.target.contains("aarch64-unknown-linux-musl") && r.target.contains("pandamux-cli")
    });

    if !has_linux_x86_server || !has_linux_x86_cli {
        bail!("Resources must bundle Linux x86_64 musl node binaries for remote bootstrap");
    }
    if !has_linux_arm_server || !has_linux_arm_cli {
        bail!("Resources must bundle Linux aarch64 musl node binaries for remote bootstrap");
    }

    // 4. Verify NSIS currentUser configuration
    let nsis = config
        .nsis
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing [package.metadata.packager.nsis] section"))?;

    if nsis.installer_mode != "currentUser" {
        bail!(
            "NSIS installer-mode must be 'currentUser', found '{}'",
            nsis.installer_mode
        );
    }

    if !nsis.appdata_paths.iter().any(|p| p.contains("pandamux")) {
        bail!("NSIS appdata-paths must specify '$LOCALAPPDATA/pandamux'");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_validate_fixture() {
        let fixture = include_str!("../fixtures/packager_config.toml");
        let config = parse_packager_config(fixture).expect("Must parse packager fixture");
        assert_eq!(config.product_name, "PandaMUX");
        assert_eq!(config.identifier, "com.pandamux.app");
        assert_eq!(config.binaries.len(), 3);
        assert!(config.before_packaging_command.is_none());

        validate_packaging_requirements(&config).expect("Fixture must meet packaging requirements");
    }

    #[test]
    fn test_rejects_before_packaging_command() {
        let mut config = parse_packager_config(include_str!("../fixtures/packager_config.toml")).unwrap();
        config.before_packaging_command = Some("cargo build --release".to_string());
        let res = validate_packaging_requirements(&config);
        assert!(res.is_err(), "Must reject before-packaging-command to protect signatures");
    }
}
