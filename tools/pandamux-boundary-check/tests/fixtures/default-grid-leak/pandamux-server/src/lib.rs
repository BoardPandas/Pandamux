pub fn fixture() -> (&'static str, &'static str) {
    (pandamux_term::grid(), pandamux_term::pty())
}
