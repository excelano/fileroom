// Every conformant fixture's table, written back by the module that reads
// it, reads as the same value.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod common;

use std::io::Cursor;

use common::{examples, pack};
use fileroom::records::{self, Reading, Table};
use fileroom::slpc::toml_edit::DocumentMut;
use fileroom::slpc::Container;

fn valid_cases() -> Vec<String> {
    let text = std::fs::read_to_string(examples().join("manifest.toml")).unwrap();
    let doc: DocumentMut = text.parse().unwrap();
    doc["case"]
        .as_array_of_tables()
        .unwrap()
        .iter()
        .filter(|t| t["verdict"].as_str() == Some("conformant"))
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .filter(|n| n != "register")
        .collect()
}

#[test]
fn every_conformant_table_round_trips_through_its_writer() {
    for name in valid_cases() {
        let bytes = pack(&examples().join("valid").join(&name));
        let c = Container::read(Cursor::new(bytes)).unwrap();
        let Reading::Table(table) = records::read(c.flyleaf()) else {
            panic!("{name} does not read");
        };
        let written = match &table {
            Table::Record(r) => r.to_toml(),
            Table::Hold(h) => h.to_toml(),
            Table::Aggregation(a) => a.to_toml(),
            Table::DisposalBatch(_) => continue,
        };
        let doc = records::flyleaf("x", written);
        let text = doc.to_string();
        let again: DocumentMut = text.parse().unwrap();
        match records::read(&again) {
            Reading::Table(t) => assert_eq!(t, table, "{name}:\n{text}"),
            other => panic!("{name}: {other:?}\n{text}"),
        }
    }
}
