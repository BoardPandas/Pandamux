# PandaMUX Rust 1.89 compatibility patch

This directory vendors the library inputs from `gpui-base` 0.6.6 on
crates.io, whose published archive checksum is
`c9e904ee0d351f4e85b967e26ae986d3aa195a8d9d35fcd18184a2c40989694e`.
It remains Apache-2.0 licensed under the included `LICENSE-APACHE` file. The
published benchmarks, tests, original workspace manifest, and crate-local
lockfile are omitted because Cargo does not build dependency-owned targets;
the normalized manifest, library sources, upstream README, and license remain
byte-identical before the change below.

The only intentional upstream source change replaces the post-Rust-1.89
`AtomicU64::try_update()` call with the stable `AtomicU64::fetch_update()`
equivalent. Both operations atomically apply the same checked increment and
return the previous value, preserving the exact `gpui-base` 0.6.6 public API
and behavior while keeping PandaMUX's declared Rust 1.89 MSRV enforceable.

Remove the `[patch.crates-io]` entry and this directory only after the pinned
GPUI dependency family publishes and passes an equivalent Rust
1.89-compatible release across PandaMUX's full target matrix.
