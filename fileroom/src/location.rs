//! Where a container is, said so that every platform agrees (SPEC §2.8.1).
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::Path;

use slpc::toml_edit::{InlineTable, TableLike, Value};
use unicode_normalization::UnicodeNormalization;

use crate::keys::Keys;
use crate::settings::Settings;
use crate::Malformed;

const RULE: &str = "2.8.1";

/// A platform the settings name a root's path for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Paths are UNC or drive-lettered, `\`-separated, compared without case.
    Windows,
    /// Paths are `/`-separated and compared without case.
    Macos,
    /// Paths are `/`-separated and compared as written.
    Linux,
}

impl Platform {
    /// The platform this program is running on, where it is one of the three.
    #[must_use]
    pub fn host() -> Option<Self> {
        if cfg!(windows) {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }

    fn folds_case(self) -> bool {
        !matches!(self, Self::Linux)
    }

    fn components(self, path: &str) -> Vec<String> {
        let path = match self {
            Self::Windows => strip_verbatim(path).replace('/', "\\"),
            _ => path.to_owned(),
        };
        let separator = if self == Self::Windows { '\\' } else { '/' };
        let mut parts: Vec<String> = path.split(separator).map(str::to_owned).collect();
        while parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
            parts.pop();
        }
        parts
    }

    fn same(self, a: &str, b: &str) -> bool {
        if self.folds_case() {
            a.to_lowercase() == b.to_lowercase()
        } else {
            a == b
        }
    }
}

/// `\\?\UNC\server\share\p` to `\\server\share\p`, `\\?\C:\p` to `C:\p`.
fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        path.to_owned()
    }
}

/// Where a container was when an entry was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The share root the container was under, by the name the settings give it.
    pub root: Option<String>,
    /// The path relative to the root, `/`-separated and NFC; present with `root`.
    pub path: Option<String>,
    /// The path as the operating system reported it.
    pub raw: String,
}

impl Location {
    /// Whether two locations are the same place, or `None` where either has no
    /// root and the question has no answer.
    #[must_use]
    pub fn same_as(&self, other: &Self) -> Option<bool> {
        match (&self.root, &other.root) {
            (Some(a), Some(b)) => Some(a == b && self.path == other.path),
            _ => None,
        }
    }

    /// Read the table an entry carries.
    ///
    /// # Errors
    ///
    /// A key missing, of the wrong type, or `path` present without `root`.
    pub fn from_table(table: &dyn TableLike, at: &str) -> Result<Self, Malformed> {
        let k = Keys::new(table, at);
        let root = k.optional_string(RULE, "root")?.map(str::to_owned);
        let path = k.optional_string(RULE, "path")?.map(str::to_owned);
        if root.is_some() != path.is_some() {
            return Err(Malformed::new(
                RULE,
                k.path("path"),
                "present when and only when root is",
            ));
        }
        Ok(Self {
            root,
            path,
            raw: k.string(RULE, "raw")?.to_owned(),
        })
    }

    /// The inline table an entry carries.
    #[must_use]
    pub fn to_table(&self) -> InlineTable {
        let mut t = InlineTable::new();
        if let (Some(root), Some(path)) = (&self.root, &self.path) {
            t.insert("root", Value::from(root.as_str()));
            t.insert("path", Value::from(path.as_str()));
        }
        t.insert("raw", Value::from(self.raw.as_str()));
        t
    }
}

/// The share roots as this host reaches them.
///
/// Built once from the settings: each root's path for the host platform,
/// canonicalized where the share is reachable so that a mapped drive letter
/// on Windows resolves to the UNC path it stands for.
#[derive(Debug, Clone)]
pub struct Mounts {
    platform: Option<Platform>,
    roots: Vec<(String, Vec<String>)>,
}

impl Mounts {
    /// The roots for the host platform.
    #[must_use]
    pub fn of(settings: &Settings) -> Self {
        let platform = Platform::host();
        let declared = platform.map_or_else(Vec::new, |p| {
            settings
                .roots
                .iter()
                .filter_map(|(name, root)| root.on(p).map(|path| (name.clone(), path.to_owned())))
                .collect()
        });
        let roots = declared
            .into_iter()
            .map(|(name, path)| {
                let reached = std::fs::canonicalize(&path)
                    .map(|c| c.to_string_lossy().into_owned())
                    .unwrap_or(path);
                (name, reached)
            })
            .collect();
        Self::on(platform, roots)
    }

    /// Roots for a given platform, from each root's path there, for a caller
    /// resolving paths recorded on another platform.
    #[must_use]
    pub fn on(platform: Option<Platform>, roots: Vec<(String, String)>) -> Self {
        let mut roots: Vec<(String, Vec<String>)> = platform.map_or_else(Vec::new, |p| {
            roots
                .into_iter()
                .map(|(name, path)| (name, p.components(&path)))
                .collect()
        });
        roots.sort_by_key(|(_, parts)| std::cmp::Reverse(parts.len()));
        Self { platform, roots }
    }

    /// The location of a container on this host.
    ///
    /// # Errors
    ///
    /// The path cannot be canonicalized, which means it cannot be reached.
    pub fn locate(&self, path: &Path) -> std::io::Result<Location> {
        let raw = std::path::absolute(path)?.to_string_lossy().into_owned();
        let canonical = std::fs::canonicalize(path)?;
        let resolved = self.resolve(&canonical.to_string_lossy());
        Ok(Location {
            root: resolved.as_ref().map(|(root, _)| root.clone()),
            path: resolved.map(|(_, path)| path),
            raw,
        })
    }

    /// The root and relative path of a canonical path, where a root covers it.
    #[must_use]
    pub fn resolve(&self, canonical: &str) -> Option<(String, String)> {
        let platform = self.platform?;
        let parts = platform.components(canonical);
        self.roots.iter().find_map(|(name, root)| {
            let under = parts.len() >= root.len()
                && root.iter().zip(&parts).all(|(a, b)| platform.same(a, b));
            under.then(|| {
                let rest: String = parts[root.len()..].join("/");
                (name.clone(), rest.nfc().collect())
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legal(platform: Platform, declared: &str) -> Mounts {
        Mounts::on(
            Some(platform),
            vec![
                ("legal".to_owned(), declared.to_owned()),
                (
                    "legal-archive".to_owned(),
                    format!(
                        "{declared}{}archive",
                        if platform == Platform::Windows {
                            '\\'
                        } else {
                            '/'
                        }
                    ),
                ),
            ],
        )
    }

    #[test]
    fn one_share_three_platforms() {
        let want = Some((
            "legal".to_owned(),
            "contracts/C-4471/invoice.pdf.slpc".to_owned(),
        ));
        let windows = legal(Platform::Windows, r"\\files\legal");
        assert_eq!(
            windows.resolve(r"\\?\UNC\files\legal\contracts\C-4471\invoice.pdf.slpc"),
            want
        );
        assert_eq!(
            windows.resolve(r"\\FILES\Legal\contracts\C-4471\invoice.pdf.slpc"),
            want
        );
        let macos = legal(Platform::Macos, "/Volumes/legal");
        assert_eq!(
            macos.resolve("/Volumes/legal/contracts/C-4471/invoice.pdf.slpc"),
            want
        );
        let linux = legal(Platform::Linux, "/mnt/legal");
        assert_eq!(
            linux.resolve("/mnt/legal/contracts/C-4471/invoice.pdf.slpc"),
            want
        );
        assert_eq!(
            linux.resolve("/mnt/Legal/contracts/C-4471/invoice.pdf.slpc"),
            None
        );
    }

    #[test]
    fn mapped_drive_is_its_unc_root() {
        assert_eq!(strip_verbatim(r"\\?\UNC\files\legal\a"), r"\\files\legal\a");
        assert_eq!(strip_verbatim(r"\\?\C:\a"), r"C:\a");
        let windows = legal(Platform::Windows, r"\\files\legal");
        assert_eq!(windows.resolve(r"Z:\contracts\a.slpc"), None);
    }

    #[test]
    fn longest_root_wins_and_path_is_nfc() {
        let linux = legal(Platform::Linux, "/mnt/legal");
        assert_eq!(
            linux.resolve("/mnt/legal/archive/2019/e\u{0301}te\u{0301}.slpc"),
            Some((
                "legal-archive".to_owned(),
                "2019/\u{e9}t\u{e9}.slpc".to_owned()
            ))
        );
        assert_eq!(
            linux.resolve("/mnt/legal"),
            Some(("legal".to_owned(), String::new()))
        );
        assert_eq!(linux.resolve("/mnt/legalese/x"), None);
    }

    #[test]
    fn locations_compare_by_root_and_path() {
        let at = |root: Option<&str>, path: Option<&str>| Location {
            root: root.map(str::to_owned),
            path: path.map(str::to_owned),
            raw: String::new(),
        };
        assert_eq!(
            at(Some("legal"), Some("a")).same_as(&at(Some("legal"), Some("a"))),
            Some(true)
        );
        assert_eq!(
            at(Some("legal"), Some("a")).same_as(&at(Some("legal"), Some("A"))),
            Some(false)
        );
        assert_eq!(at(Some("legal"), Some("a")).same_as(&at(None, None)), None);
    }

    #[test]
    fn table_round_trip() {
        let loc = Location {
            root: Some("legal".into()),
            path: Some("a/b.slpc".into()),
            raw: r"\\files\legal\a\b.slpc".into(),
        };
        let table = loc.to_table();
        assert_eq!(Location::from_table(&table, "location").unwrap(), loc);
        let mut lone = InlineTable::new();
        lone.insert("path", Value::from("a"));
        lone.insert("raw", Value::from("a"));
        assert_eq!(
            Location::from_table(&lone, "location").unwrap_err().rule,
            "2.8.1"
        );
    }
}
