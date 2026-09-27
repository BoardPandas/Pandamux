use anyhow::{bail, Result};
use crate::config::PackagerMetadata;

pub struct NsisScriptGenerator;

impl NsisScriptGenerator {
    pub fn generate_script(
        config: &PackagerMetadata,
        stage_dir_var: &str,
        version: &str,
    ) -> Result<String> {
        let nsis = config
            .nsis
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("NSIS configuration section is required"))?;

        if nsis.installer_mode != "currentUser" {
            bail!("Only currentUser installer mode is supported for PandaMUX NSIS generation");
        }

        let mut script = String::new();
        script.push_str("; ============================================================\n");
        script.push_str("; PandaMUX NSIS Installer Script (cargo-packager generated)\n");
        script.push_str("; Mode: currentUser ($LOCALAPPDATA\\Programs\\PandaMUX)\n");
        script.push_str("; ============================================================\n\n");

        script.push_str("Unicode true\n");
        script.push_str("RequestExecutionLevel user\n");
        script.push_str("SetCompressor /SOLID lzma\n\n");

        script.push_str(&format!("!define PRODUCT_NAME \"{}\"\n", config.product_name));
        script.push_str(&format!("!define PRODUCT_VERSION \"{}\"\n", version));
        if let Some(ref pub_name) = config.publisher {
            script.push_str(&format!("!define PRODUCT_PUBLISHER \"{}\"\n", pub_name));
        }
        if let Some(ref url) = config.homepage {
            script.push_str(&format!("!define PRODUCT_WEB_SITE \"{}\"\n", url));
        }

        script.push_str("\nInstallDir \"$LOCALAPPDATA\\Programs\\PandaMUX\"\n\n");

        // Section: Main install
        script.push_str("Section \"MainSection\" SEC01\n");
        script.push_str("    SetOutPath \"$INSTDIR\"\n");
        script.push_str("    SetOverwrite on\n\n");

        // Multi-binary installation
        script.push_str("    ; Multi-binary payload\n");
        for binary in &config.binaries {
            let filename = std::path::Path::new(&binary.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("app.exe");
            script.push_str(&format!("    File \"${{{}}}\\{}\"\n", stage_dir_var, filename));
        }
        script.push_str("\n");

        // Bundled resources installation
        script.push_str("    ; Bundled resources\n");
        for resource in &config.resources {
            let target_path = resource.target.replace('/', "\\");
            script.push_str(&format!("    SetOutPath \"$INSTDIR\\{}\"\n", target_path));
            let src_path = resource.src.replace('/', "\\");
            script.push_str(&format!(
                "    File /r \"${{{}}}\\{}\\*.*\"\n",
                stage_dir_var, src_path
            ));
        }
        script.push_str("\n");

        // Shortcuts and Uninstall registration
        script.push_str("    ; Shortcuts and Registry (HKCU)\n");
        script.push_str("    CreateDirectory \"$SMPROGRAMS\\PandaMUX\"\n");
        script.push_str("    CreateShortcut \"$SMPROGRAMS\\PandaMUX\\PandaMUX.lnk\" \"$INSTDIR\\pandamux.exe\"\n");
        script.push_str("    WriteUninstaller \"$INSTDIR\\uninstall.exe\"\n\n");

        script.push_str("    WriteRegStr HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX\" \"DisplayName\" \"PandaMUX\"\n");
        script.push_str("    WriteRegStr HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX\" \"UninstallString\" '\"$INSTDIR\\uninstall.exe\"'\n");
        script.push_str("    WriteRegStr HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX\" \"DisplayIcon\" \"$INSTDIR\\pandamux.exe\"\n");
        script.push_str("    WriteRegStr HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX\" \"DisplayVersion\" \"${PRODUCT_VERSION}\"\n");
        if let Some(ref pub_name) = config.publisher {
            script.push_str(&format!(
                "    WriteRegStr HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX\" \"Publisher\" \"{}\"\n",
                pub_name
            ));
        }
        script.push_str("SectionEnd\n\n");

        // Section: Uninstall
        script.push_str("Section \"Uninstall\"\n");
        script.push_str("    Delete \"$SMPROGRAMS\\PandaMUX\\PandaMUX.lnk\"\n");
        script.push_str("    RMDir \"$SMPROGRAMS\\PandaMUX\"\n\n");

        for binary in &config.binaries {
            let filename = std::path::Path::new(&binary.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("app.exe");
            script.push_str(&format!("    Delete \"$INSTDIR\\{}\"\n", filename));
        }
        script.push_str("    Delete \"$INSTDIR\\uninstall.exe\"\n\n");
        script.push_str("    RMDir /r \"$INSTDIR\\resources\"\n");
        script.push_str("    RMDir \"$INSTDIR\"\n\n");
        script.push_str("    DeleteRegKey HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX\"\n");
        script.push_str("SectionEnd\n");

        Ok(script)
    }

    pub fn validate_script(script: &str) -> Result<()> {
        if !script.contains("RequestExecutionLevel user") {
            bail!("NSIS script missing 'RequestExecutionLevel user' for currentUser mode");
        }
        if !script.contains("InstallDir \"$LOCALAPPDATA\\Programs\\PandaMUX\"") {
            bail!("NSIS script must install to '$LOCALAPPDATA\\Programs\\PandaMUX'");
        }
        if !script.contains("pandamux.exe") {
            bail!("NSIS script missing desktop binary 'pandamux.exe'");
        }
        if !script.contains("pandamux-server.exe") {
            bail!("NSIS script missing server binary 'pandamux-server.exe'");
        }
        if !script.contains("pandamux-cli.exe") {
            bail!("NSIS script missing CLI binary 'pandamux-cli.exe'");
        }
        if !script.contains("x86_64-unknown-linux-musl") {
            bail!("NSIS script missing Linux x86_64 musl resource bundling");
        }
        if !script.contains("aarch64-unknown-linux-musl") {
            bail!("NSIS script missing Linux aarch64 musl resource bundling");
        }
        if !script.contains("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX") {
            bail!("NSIS script missing HKCU Uninstall registry registration");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_packager_config;

    #[test]
    fn test_generate_and_validate_nsis_script() {
        let fixture = include_str!("../fixtures/packager_config.toml");
        let config = parse_packager_config(fixture).expect("Parse config");
        let script = NsisScriptGenerator::generate_script(&config, "STAGE_DIR", "0.53.10")
            .expect("Generate script");

        NsisScriptGenerator::validate_script(&script).expect("Validate generated script");
    }
}
