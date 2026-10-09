pub fn fixture() -> (&'static str, &'static str) {
    (gpui_pre::fixture(), gpui_pre_platform::fixture())
}
