# fileroom

The open core of Fileroom, records management on Slipcase containers: a Rust
crate that reads and verifies what the [Slipcase Records
Profile](https://github.com/excelano/slipcase-profiles/tree/main/records)
defines. A record is a `.slpc` container whose flyleaf carries a `[records]`
table and whose members carry its event log and components; this crate reads
that table, checks a container against the profile's rules, reads the
organization's settings and resolves where a container is in terms every
platform agrees on.

```no_run
fn main() -> fileroom::Result<()> {
    let mut c = fileroom::slpc::Container::open("invoice.pdf.slpc")?;
    match fileroom::records::check(&mut c)? {
        fileroom::records::Reading::Table(fileroom::records::Table::Record(r)) => {
            println!("{} captured {}", r.id, r.captured);
        }
        other => println!("{other:?}"),
    }
    Ok(())
}
```

The specification is the authority on what conforms; where this crate and the
specification disagree, the specification wins and the crate has a bug.
