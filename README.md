# fileroom

The open core of [Fileroom](https://github.com/excelano/fileroom): the
`fileroom` crate and command implement the [Slipcase Records
Profile](https://github.com/excelano/slipcase-profiles/tree/main/records),
so that a record's retention state, its history, and the disposition register
can be read and verified by anyone, with no license and no vendor.

The crate's own README, in `fileroom/`, is its documentation and its page on
crates.io. `AUDITING.md` tells an auditor or counsel how a record is found
eligible and what destruction does. `tools/corpus` generates a deterministic test corpus with a records
root and the ground truth for evaluating it; it is a workspace member and is
never published. `spec/` is the specification repository as a submodule, which
the tests read, so clone with `--recurse-submodules`.
