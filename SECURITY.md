# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately, using the **Report a vulnerability** button on the Security tab of the repository concerned. If that button is not there, or you would rather not go through GitHub, email david.anderson@excelano.com instead. I aim to respond within seven days.

Please do not open public issues for security problems.

## Supported versions

The latest release receives security fixes. Older versions are not supported.
A fix ships in a new release; update the way you installed.

## What fileroom can access

fileroom is a Rust crate and, behind its `cli` feature, the `fileroom` command.
Both run locally, make no network call of any kind, have no account and no
telemetry, and can read and write only what your operating-system user already
can. Containers are read and written through `slpc`, whose own security policy
in `excelano/slpc-rust` covers the container format.

The crate reads and writes the paths its caller hands it and nothing else. The
command reads the containers named on its command line, and directories of them
with `--recursive`, and a records root named by `--root` or `FILEROOM_ROOT`: its
`settings.toml`, the schedule under `schedule/`, the hold matters under `holds/`
and the disposition register under `register/`. `show`, `log`, `check`,
`schedule`, `register` and `verify` only read.

`fixity` writes a sweep report into the root's `fixity/` directory, never
replacing an existing one, and appends a `fixity_failed` or `moved_detected`
event to each record whose hashes or location changed. An event is appended by
writing a new container beside the record and renaming it over the record once
it is complete. Where `settings.toml` sets `operator_identity = "verified"`, the
command has no verified operator, so it reports and appends nothing. The
`--actor` it records is taken as stated and not verified.

**Only the `dispose` feature deletes.** It is off by default, the Debian
package is built without it, and a library consumer that does not ask for it
links nothing that can destroy a file. With it, `plan` writes a plan to the
file given by `--out` and never replaces one. `dispose` reads an approved plan,
asks for `yes` on the terminal unless given `--yes`, and deletes the containers
the plan lists, each one checked against the hold matters first. It records
the batch in the root's `register/` as an intent, a journal and a final
container, holding a lock file there while it runs, and the intent carries this
machine's host name and the `USER` or `USERNAME` it runs as. The approvers
named by `--approved-by` are recorded as stated and not verified. `recover`
finishes an unfinished batch in the register from its journal and never
deletes.

## What fileroom stores

Nothing beyond the files described above. There is no configuration
directory, no history file, no cache, no telemetry, and no remote logging.
Every file it writes is under a records root or at a path its caller named, and
the temporary file a container is rewritten through sits beside that container
and is removed if the write fails.

## Verifying releases

Every GitHub release carries the `.deb` packages, built by `build-release`, the
release program in excelano/shipping, from the tagged commit. The release
attaches no checksum file, so a download from it is vouched for only by where
it came from. Packages served from the Excelano apt repository are signed;
https://excelano.com/apt/ has the key and the one-line setup. The crate on
crates.io is checked by `cargo` against the checksum in the registry's index.
