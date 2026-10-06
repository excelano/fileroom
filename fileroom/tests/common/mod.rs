// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#![allow(dead_code)]

use std::io::Cursor;
use std::path::{Path, PathBuf};

use fileroom::slpc::toml_edit::DocumentMut;
use fileroom::slpc::Repack;

pub fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/records/examples")
}

pub fn pack(dir: &Path) -> Vec<u8> {
    let flyleaf = std::fs::read(dir.join("slipcase.flyleaf.toml")).unwrap();
    let doc: DocumentMut = std::str::from_utf8(&flyleaf).unwrap().parse().unwrap();
    let content_name = doc["content"]["file"].as_str().unwrap().to_owned();
    let content = std::fs::read(dir.join(&content_name)).unwrap();
    let mut first = Cursor::new(Vec::new());
    fileroom::slpc::pack_reader(&content_name, Cursor::new(content), doc, &mut first).unwrap();

    let mut members = Vec::new();
    walk(dir, dir, &mut members);
    let bodies: Vec<(String, Vec<u8>)> = members
        .into_iter()
        .filter(|name| name != "slipcase.flyleaf.toml" && *name != content_name)
        .map(|name| {
            let bytes = std::fs::read(dir.join(&name)).unwrap();
            (name, bytes)
        })
        .collect();
    let mut out = Cursor::new(Vec::new());
    let mut repack = Repack::new(Cursor::new(first.into_inner())).flyleaf_bytes(&flyleaf);
    for (name, bytes) in &bodies {
        repack = repack.member(name, Cursor::new(bytes));
    }
    repack.write(&mut out).unwrap();
    out.into_inner()
}

fn walk(root: &Path, dir: &Path, names: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, names);
        } else {
            let rel = path.strip_prefix(root).unwrap();
            names.push(
                rel.components()
                    .map(|c| c.as_os_str().to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}
