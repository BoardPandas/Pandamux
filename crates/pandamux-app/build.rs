//! Embeds the Windows icon + version metadata into the release `pandamux.exe`
//! (Phase 7). Replaces the Electron build's rcedit step: the icon and the
//! ProductName/CompanyName/version strings are baked into the PE resource table
//! at compile time, so Explorer, the taskbar, and the properties dialog all show
//! "PandaMUX Everywhere".
//!
//! FileVersion/ProductVersion come from CARGO_PKG_VERSION automatically, so the
//! single version source (workspace Cargo.toml) drives the exe metadata too.
//!
//! A missing resource compiler (a dev box without the Windows SDK) is treated as
//! a warning, not a hard error, so local `cargo build` still succeeds without the
//! icon; CI runs on windows-latest where rc.exe is present and embeds for real.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../resources/icons/icon.ico");
        res.set("ProductName", "PandaMUX Everywhere");
        res.set("FileDescription", "PandaMUX Everywhere");
        res.set("CompanyName", "BoardPandas");
        res.set("InternalName", "pandamux");
        res.set("OriginalFilename", "pandamux.exe");
        res.set("LegalCopyright", "Copyright (c) 2026 BoardPandas");
        if let Err(error) = res.compile() {
            println!("cargo:warning=winresource icon/metadata embed skipped: {error}");
        }
    }
}
