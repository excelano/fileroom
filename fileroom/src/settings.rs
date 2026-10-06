//! The records root's `settings.toml` (SPEC §7.1).
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeMap;
use std::path::Path;

use slpc::toml_edit::DocumentMut;

use crate::keys::Keys;
use crate::location::Platform;
use crate::{Error, Malformed};

const RULE: &str = "7.1";

/// The organization's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The organization's name, as it appears on a certificate.
    pub organization: String,
    /// The month the fiscal year begins, 1 to 12.
    pub fiscal_year_start_month: u8,
    /// The share roots, by name.
    pub roots: BTreeMap<String, Root>,
}

/// One share, as each platform mounts it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Root {
    /// The path on Windows, as a UNC path.
    pub windows: Option<String>,
    /// The path on macOS.
    pub macos: Option<String>,
    /// The path on Linux.
    pub linux: Option<String>,
}

impl Root {
    /// The share's path on a platform, where it has one.
    #[must_use]
    pub fn on(&self, platform: Platform) -> Option<&str> {
        match platform {
            Platform::Windows => self.windows.as_deref(),
            Platform::Macos => self.macos.as_deref(),
            Platform::Linux => self.linux.as_deref(),
        }
    }
}

impl Settings {
    /// Read settings from a file.
    ///
    /// # Errors
    ///
    /// The file cannot be read, is not TOML, or breaks §7.1.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    /// Read settings from their text.
    ///
    /// # Errors
    ///
    /// Not TOML, or breaks §7.1.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let doc: DocumentMut = text.parse()?;
        let k = Keys::new(doc.as_table(), "");
        let month = k.integer(RULE, "fiscal_year_start_month")?;
        let fiscal_year_start_month = u8::try_from(month)
            .ok()
            .filter(|m| (1..=12).contains(m))
            .ok_or_else(|| {
                Malformed::new(
                    RULE,
                    "fiscal_year_start_month",
                    format!("{month} is not a month"),
                )
            })?;
        let mut roots = BTreeMap::new();
        if let Some(table) = k.optional_table(RULE, "roots")? {
            for (name, _) in table.entries() {
                if name.is_empty() {
                    return Err(Malformed::new(RULE, "roots", "a root has an empty name").into());
                }
                let r = table.table(RULE, name)?;
                let mut root = Root::default();
                for (platform, _) in r.entries() {
                    let value = Some(r.string(RULE, platform)?.to_owned());
                    match platform {
                        "windows" => root.windows = value,
                        "macos" => root.macos = value,
                        "linux" => root.linux = value,
                        _ => {
                            return Err(Malformed::new(
                                RULE,
                                r.path(platform),
                                "not a platform: windows, macos, or linux",
                            )
                            .into())
                        }
                    }
                }
                roots.insert(name.to_owned(), root);
            }
        }
        Ok(Self {
            organization: k.string(RULE, "organization")?.to_owned(),
            fiscal_year_start_month,
            roots,
        })
    }
}
