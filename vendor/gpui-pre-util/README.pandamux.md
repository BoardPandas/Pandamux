# PandaMUX Rust 1.89 compatibility patch

This directory vendors `gpui-pre-util` 0.3.6 from crates.io, whose published
archive checksum is
`6a026a30c578930d20974fd8c3d77de36223ac7ba193bd4259e7238929bc8806`.
It remains Apache-2.0 licensed under the included `LICENSE-APACHE` file.

The only intentional upstream source change is in `TypeIdHasher::write`:
PandaMUX replaces the post-Rust-1.89 `slice::as_array()` call with the stable
`TryInto<&[u8; 8]>` equivalent. This preserves the exact `gpui-pre` 0.3.6 API
and behavior while keeping PandaMUX's declared Rust 1.89 MSRV enforceable.

Remove the `[patch.crates-io]` entry and this directory only after the pinned
GPUI dependency family publishes and passes an equivalent Rust 1.89-compatible
release across PandaMUX's full target matrix.
