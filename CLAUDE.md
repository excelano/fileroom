# fileroom

The `fileroom` crate: the Slipcase Records Profile's formats, and behind the
`dispose` feature, disposition. `spec/` is the `slipcase-profiles` repository
as a submodule; `spec/records/SPEC.md` is what the code implements and
`spec/records/examples/` is what the tests read, so clone with
`--recurse-submodules` or run `git submodule update --init` first.

Every rule check reports the section it enforces as the specification numbers
it, and the example manifest's `rule` is what the tests compare against, so a
check's rule string has to match the manifest's before a new fixture passes.

`cargo test --workspace --all-features` and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` are what
CI runs; both pass before a commit.
