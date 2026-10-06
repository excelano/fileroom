// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use fileroom::location::Platform;
use fileroom::settings::Settings;
use fileroom::Error;

const EXAMPLE: &str = r#"
organization = "Example Corporation"
fiscal_year_start_month = 10

[roots.legal]
windows = '\\files\legal'
macos = "/Volumes/legal"
linux = "/mnt/legal"
"#;

#[test]
fn the_specification_example_loads() {
    let s = Settings::parse(EXAMPLE).unwrap();
    assert_eq!(s.organization, "Example Corporation");
    assert_eq!(s.fiscal_year_start_month, 10);
    assert_eq!(
        s.roots["legal"].on(Platform::Windows),
        Some(r"\\files\legal")
    );
    assert_eq!(s.roots["legal"].on(Platform::Linux), Some("/mnt/legal"));
}

#[test]
fn a_root_without_every_platform_resolves_on_the_ones_it_names() {
    let s = Settings::parse(
        "organization = \"x\"\nfiscal_year_start_month = 1\n[roots.hr]\nlinux = \"/mnt/hr\"\n",
    )
    .unwrap();
    assert_eq!(s.roots["hr"].on(Platform::Macos), None);
    assert_eq!(s.roots["hr"].on(Platform::Linux), Some("/mnt/hr"));
}

fn refused(text: &str) -> String {
    match Settings::parse(text) {
        Err(Error::Malformed(m)) => {
            assert_eq!(m.rule, "7.1");
            m.key
        }
        other => panic!("accepted or failed otherwise: {other:?}"),
    }
}

#[test]
fn settings_rules_are_refused_by_key() {
    assert_eq!(
        refused("organization = \"x\"\nfiscal_year_start_month = 13\n"),
        "fiscal_year_start_month"
    );
    assert_eq!(refused("fiscal_year_start_month = 1\n"), "organization");
    assert_eq!(
        refused("organization = \"x\"\nfiscal_year_start_month = 1\n[roots.hr]\nmacOS = \"/Volumes/hr\"\n"),
        "roots.hr.macOS"
    );
    assert!(matches!(
        Settings::parse("organization = [\n"),
        Err(Error::Toml(_))
    ));
}
