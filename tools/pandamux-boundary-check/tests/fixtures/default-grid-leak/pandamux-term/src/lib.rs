#[cfg(feature = "grid")]
pub fn grid() -> &'static str {
    alacritty_terminal::fixture()
}

#[cfg(feature = "pty")]
pub fn pty() -> &'static str {
    portable_pty::fixture()
}
