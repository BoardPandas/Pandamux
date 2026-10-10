# PandaMUX Rust 1.89 compatibility patch

This directory vendors the library and build inputs from `gpui-pre` 0.3.6 on
crates.io, whose published archive checksum is
`a0437c0b83e636a92bd1a39fa1d05fb632ae671289537497b35871ffbe231b84`.
It remains Apache-2.0 licensed under the included `LICENSE-APACHE` file. The
published examples, tests, documentation, original workspace manifest, and
crate-local lockfile are omitted because Cargo does not build dependency-owned
targets; the normalized manifest, library sources, Windows build resources,
upstream README, and license remain byte-identical before the changes below.

The only intentional upstream source changes are:

- replace six post-Rust-1.89 `std::hint::cold_path()` calls with an equivalent
  private function carrying stable `#[cold]` and `#[inline(never)]` attributes;
- replace one post-Rust-1.89 `str::ceil_char_boundary()` call with the same
  clamped forward-boundary search implemented using stable string APIs.
- remove trailing spaces from two empty rustdoc lines so the vendored source
  satisfies PandaMUX's staged-diff whitespace gate without semantic change.

These changes preserve the exact `gpui-pre` 0.3.6 public API and observable
behavior while keeping PandaMUX's declared Rust 1.89 MSRV enforceable. Remove
the `[patch.crates-io]` entry and this directory only after the pinned GPUI
dependency family publishes and passes an equivalent Rust 1.89-compatible
release across PandaMUX's full target matrix.
