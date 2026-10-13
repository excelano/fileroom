// The demonstration root: an invented company's records in one language,
// believable enough to photograph and to hand a store reviewer, with nothing
// broken in it. The test corpus is the other half of this crate and is where
// every defect lives.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::{Path, PathBuf};

use crate::build::{Aggregation, Matter, Record, SeriesEntry};
use crate::dates::{add_days, instant, unix_ms};
use crate::rng::Rng;
use clap::ValueEnum;
use fileroom::conventions::{Agent, Date, Identifier, Instant};
use fileroom::location::Location;
use fileroom::schedule::Store;

/// The program the history names, as the invented company would have seen it.
const TOOL: &str = "slipcase-fileroom 0.1.0";

/// The language a demonstration root is written in.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Language {
    En,
    De,
}

/// A schedule row: code, title, classification, legal citation, disposition,
/// retention, retention type, event, longer retention authorized, disposal
/// action, cutoff, period kind.
type SeriesRow = [&'static str; 12];

/// How the records of one kind are named and filed.
struct Kind {
    series: &'static str,
    folder: &'static str,
    /// Indexes into the company's staff.
    staff: &'static [usize],
    /// The file stem and the content's title, with `{y}`, `{m}`, `{n}`,
    /// `{party}` and `{name}` filled in.
    stem: &'static str,
    title: &'static str,
    /// Records of this kind: one a year where `annual`, otherwise a weight.
    weight: u32,
    annual: bool,
    years: (i64, i64),
    email: bool,
    marking: bool,
    /// Days from creation to the series' event, for the share of records
    /// that have reached it; the rest await it.
    event: Option<(i64, i64, u64)>,
}

struct MatterSpec {
    kind: &'static str,
    title: &'static str,
    mandate: &'static str,
    placed: (u16, u8, u8),
    end: Option<(u16, u8, u8)>,
    released: Option<(u16, u8, u8)>,
    custodian: usize,
    created_from: Option<(u16, u8, u8)>,
    created_before: Option<(u16, u8, u8)>,
}

struct ContractSpec {
    title: &'static str,
    started: (u16, u8, u8),
    closed: Option<(u16, u8, u8)>,
}

struct Company {
    organization: &'static str,
    fiscal_year_start_month: u8,
    share: &'static str,
    domain: &'static str,
    staff: &'static [&'static str],
    records_manager: &'static str,
    counsel: &'static str,
    schedule_source: &'static str,
    series: &'static [SeriesRow],
    kinds: &'static [Kind],
    parties: &'static [&'static str],
    names: &'static [&'static str],
    subjects: &'static [&'static str],
    months: [&'static str; 12],
    matters: &'static [MatterSpec],
    contracts_title: &'static str,
    contracts: &'static [ContractSpec],
    contract_series: &'static str,
    contract_folder: &'static str,
    contract_custodian: usize,
    contract_documents: &'static [(&'static str, &'static str)],
    marking: &'static str,
    footer: &'static str,
}

const EN: Company = Company {
    organization: "Calder Freight Co.",
    fiscal_year_start_month: 7,
    share: "files",
    domain: "calderfreight.example",
    staff: &[
        "maria.okafor",
        "daniel.reyes",
        "priya.shah",
        "tom.becker",
        "hannah.lund",
        "samuel.ortiz",
    ],
    records_manager: "grace.whitfield",
    counsel: "ruth.adeyemi",
    schedule_source: "Calder Freight records retention schedule, 2026 edition",
    series: &[
        ["FIN-100", "Accounts payable invoices and receipts", "Finance", "26 CFR 1.6001-1", "Temporary", "7", "Creation_Age", "", "Yes", "", "fiscal_year", ""],
        ["FIN-210", "General ledger", "Finance", "26 CFR 1.6001-1", "Temporary", "10", "Creation_Age", "", "Yes", "", "fiscal_year", ""],
        ["FIN-300", "Bank statements and reconciliations", "Finance", "", "Temporary", "7", "Creation_Age", "", "Yes", "", "fiscal_year", ""],
        ["FIN-900", "Annual financial statements", "Finance", "", "Permanent", "", "", "", "", "", "", ""],
        ["HR-100", "Payroll and employment tax records", "Human resources", "26 CFR 31.6001-1", "Temporary", "4", "Creation_Age", "", "Yes", "", "calendar_year", ""],
        ["HR-200", "Personnel files", "Human resources", "", "Temporary", "7", "Event_Age", "Separation", "Yes", "", "calendar_year", ""],
        ["HR-300", "Applications from candidates not hired", "Human resources", "29 CFR 1602.14", "Temporary", "2", "Creation_Age", "", "No", "", "none", "maximum"],
        ["OPS-100", "Bills of lading and delivery receipts", "Operations", "", "Temporary", "3", "Creation_Age", "", "Yes", "", "calendar_year", ""],
        ["SAF-100", "Injury and illness logs (OSHA 300)", "Safety", "29 CFR 1904.33", "Temporary", "5", "Creation_Age", "", "No", "", "calendar_year", ""],
        ["LEG-100", "Customer contracts", "Legal", "", "Temporary", "6", "Event_Age", "Final action", "Yes", "", "calendar_year", ""],
        ["ADM-100", "Board minutes and resolutions", "Administration", "", "Permanent", "", "", "", "", "", "", ""],
        ["ADM-200", "Policies and procedures", "Administration", "", "Temporary", "3", "Creation_Age", "", "Yes", "review", "none", ""],
        ["ADM-300", "General business correspondence", "Administration", "", "Temporary", "3", "Creation_Age", "", "Yes", "", "calendar_year", ""],
    ],
    kinds: &[
        Kind { series: "FIN-100", folder: "finance", staff: &[0, 1], stem: "invoice-{y}-{n}", title: "Invoice {y}-{n} from {party}", weight: 30, annual: false, years: (2014, 2026), email: false, marking: false, event: None },
        Kind { series: "FIN-210", folder: "finance", staff: &[1], stem: "general-ledger-fy{y}", title: "General ledger, fiscal year {y}", weight: 1, annual: true, years: (2014, 2025), email: false, marking: false, event: None },
        Kind { series: "FIN-300", folder: "finance", staff: &[1], stem: "bank-statement-{y}-{m}", title: "Bank statement, {month} {y}", weight: 10, annual: false, years: (2014, 2026), email: false, marking: false, event: None },
        Kind { series: "FIN-900", folder: "finance", staff: &[1], stem: "annual-report-fy{y}", title: "Annual financial statements, fiscal year {y}", weight: 1, annual: true, years: (2014, 2025), email: false, marking: false, event: None },
        Kind { series: "HR-100", folder: "hr", staff: &[2], stem: "payroll-register-{y}-{m}", title: "Payroll register, {month} {y}", weight: 10, annual: false, years: (2016, 2026), email: false, marking: true, event: None },
        Kind { series: "HR-200", folder: "hr", staff: &[2], stem: "personnel-file-{name}", title: "Personnel file: {name}", weight: 8, annual: false, years: (2012, 2024), email: false, marking: true, event: Some((400, 2600, 60)) },
        Kind { series: "HR-300", folder: "hr", staff: &[2], stem: "application-{name}-{y}", title: "Application: {name}, {y}", weight: 6, annual: false, years: (2023, 2026), email: false, marking: true, event: None },
        Kind { series: "OPS-100", folder: "operations", staff: &[3, 5], stem: "bill-of-lading-{y}-{n}", title: "Bill of lading {y}-{n}, {party}", weight: 25, annual: false, years: (2018, 2026), email: false, marking: false, event: None },
        Kind { series: "SAF-100", folder: "operations", staff: &[5], stem: "osha-300-log-{y}", title: "OSHA 300 log, {y}", weight: 1, annual: true, years: (2016, 2025), email: false, marking: false, event: None },
        Kind { series: "ADM-100", folder: "admin", staff: &[4], stem: "board-minutes-{y}-{m}", title: "Minutes of the board, {month} {y}", weight: 4, annual: false, years: (2014, 2026), email: false, marking: false, event: None },
        Kind { series: "ADM-200", folder: "admin", staff: &[4], stem: "policy-{party}", title: "Policy: {party}", weight: 4, annual: false, years: (2019, 2024), email: false, marking: false, event: None },
        Kind { series: "ADM-300", folder: "admin", staff: &[0, 3, 4], stem: "email-{y}{m}-{n}", title: "{subject}", weight: 12, annual: false, years: (2019, 2026), email: true, marking: false, event: None },
    ],
    parties: &[
        "Redline Tire & Service",
        "Harbor Fuel Supply",
        "Midstate Office Products",
        "Keystone Pallet Co.",
        "Ridgeway Insurance Agency",
    ],
    names: &[
        "alvarez", "brennan", "chu", "dawson", "eriksen", "fofana", "gallagher", "haddad",
        "ibarra", "jansen", "kowalski", "lindgren", "moreau", "nakamura", "oyelaran",
    ],
    subjects: &[
        "Re: Consolidated load to Louisville",
        "Re: Delivery window for week 14",
        "Pickup confirmation, Toledo dock 3",
        "Re: Fuel surcharge adjustment",
        "Re: Quote for refrigerated lanes",
    ],
    months: [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ],
    matters: &[
        MatterSpec {
            kind: "legal",
            title: "Westbrook Haulage, cargo damage claim",
            mandate: "Preserve every operations record from January 2022 on, per counsel's notice of 2026-02-06.",
            placed: (2026, 2, 9),
            end: None,
            released: None,
            custodian: 3,
            created_from: Some((2022, 1, 1)),
            created_before: None,
        },
        MatterSpec {
            kind: "extension",
            title: "IRS examination, fiscal years 2021 and 2022",
            mandate: "Keep the payables records of fiscal 2021 and 2022 until the examination closes.",
            placed: (2025, 10, 6),
            end: Some((2027, 6, 30)),
            released: None,
            custodian: 0,
            created_from: Some((2020, 7, 1)),
            created_before: Some((2022, 7, 1)),
        },
        MatterSpec {
            kind: "legal",
            title: "Former employee wage claim",
            mandate: "Preserve the personnel and payroll records named in the claim.",
            placed: (2024, 5, 13),
            end: None,
            released: Some((2025, 11, 20)),
            custodian: 2,
            created_from: None,
            created_before: None,
        },
    ],
    contracts_title: "Customer contracts",
    contracts: &[
        ContractSpec { title: "Contract 2016-014, Lakeshore Grocers", started: (2016, 4, 1), closed: Some((2019, 6, 30)) },
        ContractSpec { title: "Contract 2019-022, Pinecrest Home Supply", started: (2019, 9, 1), closed: Some((2022, 8, 31)) },
        ContractSpec { title: "Contract 2023-031, Northfield Dairy Cooperative", started: (2023, 3, 1), closed: None },
    ],
    contract_series: "LEG-100",
    contract_folder: "contracts",
    contract_custodian: 4,
    contract_documents: &[
        ("master-agreement", "Master services agreement"),
        ("rate-schedule", "Rate schedule"),
        ("amendment-1", "Amendment 1"),
        ("certificate-of-insurance", "Certificate of insurance"),
    ],
    marking: "Confidential",
    footer: "An invented document in a demonstration records root.",
};

const DE: Company = Company {
    organization: "Brunnhuber Spedition GmbH",
    fiscal_year_start_month: 1,
    share: "ablage",
    domain: "brunnhuber-spedition.example",
    staff: &[
        "anna.huber",
        "markus.weber",
        "sabine.koch",
        "tobias.maier",
        "julia.schmid",
        "stefan.wolf",
    ],
    records_manager: "claudia.berger",
    counsel: "petra.lang",
    schedule_source: "Aufbewahrungsplan der Brunnhuber Spedition, Fassung 2026",
    series: &[
        ["FIBU-01", "Buchungsbelege (Rechnungen, Quittungen)", "Finanzbuchhaltung", "§ 147 Abs. 3 AO, § 257 Abs. 4 HGB", "Temporary", "8", "Creation_Age", "", "Yes", "", "calendar_year", ""],
        ["FIBU-02", "Handelsbücher und Journale", "Finanzbuchhaltung", "§ 147 Abs. 3 AO, § 257 Abs. 4 HGB", "Temporary", "10", "Creation_Age", "", "Yes", "", "calendar_year", ""],
        ["FIBU-03", "Kontoauszüge", "Finanzbuchhaltung", "§ 147 Abs. 3 AO", "Temporary", "8", "Creation_Age", "", "Yes", "", "calendar_year", ""],
        ["FIBU-09", "Jahresabschlüsse", "Finanzbuchhaltung", "§ 257 Abs. 1 Nr. 1 HGB", "Permanent", "", "", "", "", "", "", ""],
        ["PERS-01", "Lohnkonten und Lohnabrechnungen", "Personal", "§ 41 Abs. 1 EStG", "Temporary", "6", "Creation_Age", "", "Yes", "", "calendar_year", ""],
        ["PERS-02", "Personalakten", "Personal", "§ 195 BGB", "Temporary", "3", "Event_Age", "Austritt", "Yes", "", "calendar_year", ""],
        ["PERS-03", "Unterlagen abgelehnter Bewerbungen", "Personal", "§ 15 Abs. 4 AGG", "Temporary", "6m", "Creation_Age", "", "No", "", "none", "maximum"],
        ["LOG-01", "Frachtbriefe und Lieferscheine", "Logistik", "§ 257 Abs. 1 Nr. 4 HGB", "Temporary", "8", "Creation_Age", "", "Yes", "", "calendar_year", ""],
        ["LOG-02", "Arbeitszeitnachweise der Fahrer", "Logistik", "§ 16 Abs. 2 ArbZG", "Temporary", "2", "Creation_Age", "", "No", "", "calendar_year", ""],
        ["VERT-01", "Kundenverträge", "Vertrieb", "§ 257 Abs. 1 Nr. 2 HGB", "Temporary", "6", "Event_Age", "Final action", "Yes", "", "calendar_year", ""],
        ["GF-01", "Gesellschafterbeschlüsse und Protokolle", "Geschäftsführung", "", "Permanent", "", "", "", "", "", "", ""],
        ["ORG-01", "Richtlinien und Arbeitsanweisungen", "Verwaltung", "", "Temporary", "3", "Creation_Age", "", "Yes", "review", "none", ""],
        ["KORR-01", "Handels- und Geschäftsbriefe", "Verwaltung", "§ 257 Abs. 1 Nr. 2, 3 HGB", "Temporary", "6", "Creation_Age", "", "Yes", "", "calendar_year", ""],
    ],
    kinds: &[
        Kind { series: "FIBU-01", folder: "buchhaltung", staff: &[0, 1], stem: "rechnung-{y}-{n}", title: "Eingangsrechnung {y}-{n}, {party}", weight: 30, annual: false, years: (2014, 2026), email: false, marking: false, event: None },
        Kind { series: "FIBU-02", folder: "buchhaltung", staff: &[1], stem: "hauptbuch-{y}", title: "Hauptbuch {y}", weight: 1, annual: true, years: (2014, 2025), email: false, marking: false, event: None },
        Kind { series: "FIBU-03", folder: "buchhaltung", staff: &[1], stem: "kontoauszug-{y}-{m}", title: "Kontoauszug {month} {y}", weight: 10, annual: false, years: (2014, 2026), email: false, marking: false, event: None },
        Kind { series: "FIBU-09", folder: "buchhaltung", staff: &[1], stem: "jahresabschluss-{y}", title: "Jahresabschluss {y}", weight: 1, annual: true, years: (2014, 2025), email: false, marking: false, event: None },
        Kind { series: "PERS-01", folder: "personal", staff: &[2], stem: "lohnabrechnung-{y}-{m}", title: "Lohnabrechnungen {month} {y}", weight: 10, annual: false, years: (2016, 2026), email: false, marking: true, event: None },
        Kind { series: "PERS-02", folder: "personal", staff: &[2], stem: "personalakte-{name}", title: "Personalakte {name}", weight: 8, annual: false, years: (2012, 2024), email: false, marking: true, event: Some((400, 2600, 60)) },
        Kind { series: "PERS-03", folder: "personal", staff: &[2], stem: "bewerbung-{name}-{y}", title: "Bewerbung {name}, {y}", weight: 6, annual: false, years: (2025, 2026), email: false, marking: true, event: None },
        Kind { series: "LOG-01", folder: "logistik", staff: &[3, 5], stem: "frachtbrief-{y}-{n}", title: "Frachtbrief {y}-{n}, {party}", weight: 25, annual: false, years: (2016, 2026), email: false, marking: false, event: None },
        Kind { series: "LOG-02", folder: "logistik", staff: &[5], stem: "arbeitszeit-fahrer-{y}-{m}", title: "Arbeitszeitnachweise der Fahrer, {month} {y}", weight: 4, annual: false, years: (2022, 2026), email: false, marking: false, event: None },
        Kind { series: "GF-01", folder: "verwaltung", staff: &[4], stem: "gesellschafterversammlung-{y}-{m}", title: "Protokoll der Gesellschafterversammlung, {month} {y}", weight: 4, annual: false, years: (2014, 2026), email: false, marking: false, event: None },
        Kind { series: "ORG-01", folder: "verwaltung", staff: &[4], stem: "richtlinie-{party}", title: "Richtlinie: {party}", weight: 4, annual: false, years: (2019, 2024), email: false, marking: false, event: None },
        Kind { series: "KORR-01", folder: "verwaltung", staff: &[0, 3, 4], stem: "email-{y}{m}-{n}", title: "{subject}", weight: 12, annual: false, years: (2019, 2026), email: true, marking: false, event: None },
    ],
    parties: &[
        "Reifen Brandl GmbH",
        "Tankstelle Obermeier",
        "Bürobedarf Hartl",
        "Paletten Gruber KG",
        "Versicherungsbüro Lechner",
    ],
    names: &[
        "bauer", "brandstetter", "eder", "fuchs", "graf", "hofmann", "kaiser", "lehner", "moser",
        "neumann", "pichler", "reiter", "seidl", "vogl", "winkler",
    ],
    subjects: &[
        "AW: Sammelladung nach Nürnberg",
        "AW: Lieferfenster KW 14",
        "Abholbestätigung, Rampe 3",
        "AW: Anpassung des Dieselzuschlags",
        "AW: Angebot für Kühltransporte",
    ],
    months: [
        "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September",
        "Oktober", "November", "Dezember",
    ],
    matters: &[
        MatterSpec {
            kind: "legal",
            title: "Spedition Albers, Schadensersatz wegen Transportschaden",
            mandate: "Alle Unterlagen der Logistik ab Januar 2022 aufbewahren, nach Anweisung der Rechtsabteilung vom 6. Februar 2026.",
            placed: (2026, 2, 9),
            end: None,
            released: None,
            custodian: 3,
            created_from: Some((2022, 1, 1)),
            created_before: None,
        },
        MatterSpec {
            kind: "extension",
            title: "Betriebsprüfung 2021 bis 2023",
            mandate: "Buchungsbelege der Jahre 2021 bis 2023 bis zum Abschluss der Betriebsprüfung aufbewahren.",
            placed: (2025, 10, 6),
            end: Some((2027, 12, 31)),
            released: None,
            custodian: 0,
            created_from: Some((2021, 1, 1)),
            created_before: Some((2024, 1, 1)),
        },
        MatterSpec {
            kind: "legal",
            title: "Arbeitsgerichtsverfahren, durch Vergleich beendet",
            mandate: "Die in der Klage genannten Personal- und Lohnunterlagen aufbewahren.",
            placed: (2024, 5, 13),
            end: None,
            released: Some((2025, 11, 20)),
            custodian: 2,
            created_from: None,
            created_before: None,
        },
    ],
    contracts_title: "Kundenverträge",
    contracts: &[
        ContractSpec { title: "Vertrag 2016-07, Bäckerei Lindner", started: (2016, 4, 1), closed: Some((2019, 6, 30)) },
        ContractSpec { title: "Vertrag 2019-12, Möbelhaus Kranz", started: (2019, 9, 1), closed: Some((2022, 8, 31)) },
        ContractSpec { title: "Vertrag 2023-04, Getränke Seidl", started: (2023, 3, 1), closed: None },
    ],
    contract_series: "VERT-01",
    contract_folder: "vertrieb",
    contract_custodian: 4,
    contract_documents: &[
        ("rahmenvertrag", "Rahmenvertrag"),
        ("preisblatt", "Preisblatt"),
        ("nachtrag-1", "Nachtrag 1"),
        ("versicherungsnachweis", "Versicherungsnachweis"),
    ],
    marking: "Vertraulich",
    footer: "Ein erfundenes Dokument in einer Aktenwurzel zur Vorführung.",
};

const POLICIES_EN: &[&str] = &[
    "Travel and expenses",
    "Vehicle use",
    "Remote work",
    "Records retention",
    "Driver safety",
];
const POLICIES_DE: &[&str] = &[
    "Reisekosten",
    "Dienstwagen",
    "Mobiles Arbeiten",
    "Aufbewahrung von Unterlagen",
    "Fahrersicherheit",
];
const LANES_EN: &[&str] = &[
    "Columbus to Louisville",
    "Toledo to Pittsburgh",
    "Indianapolis to Chicago",
    "Dayton to Detroit",
];
const LANES_DE: &[&str] = &[
    "Regensburg nach Nürnberg",
    "Passau nach München",
    "Landshut nach Linz",
    "Straubing nach Augsburg",
];

fn date((y, m, d): (u16, u8, u8)) -> Date {
    Date::new(y, m, d).expect("a date written by hand")
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Lowercase ASCII for a file name: umlauts spelled out, anything else
/// that is not a letter or digit becomes a hyphen.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ if !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    out.trim_matches('-').to_owned()
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A one-page PDF of the document's heading, in the standard Helvetica with
/// WinAnsi encoding so German text renders, padded with comment lines to the
/// size a scanned page would have.
fn pdf(organization: &str, title: &str, created: Date, footer: &str, padding: usize) -> Vec<u8> {
    fn text(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for c in s.chars() {
            match c {
                '(' | ')' | '\\' => {
                    out.push(b'\\');
                    out.push(c as u8);
                }
                '–' => out.push(0x96),
                '’' => out.push(0x92),
                c if (c as u32) < 256 => out.push(c as u32 as u8),
                _ => out.push(b'?'),
            }
        }
        out
    }
    let mut stream = Vec::new();
    for (size, y, line) in [
        (11, 790, organization.to_owned()),
        (18, 740, title.to_owned()),
        (11, 715, created.to_string()),
        (9, 60, footer.to_owned()),
    ] {
        stream.extend_from_slice(format!("BT /F1 {size} Tf 56 {y} Td (").as_bytes());
        stream.extend_from_slice(&text(&line));
        stream.extend_from_slice(b") Tj ET\n");
    }
    let objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>".to_vec(),
        [
            format!("<</Length {}>>\nstream\n", stream.len()).into_bytes(),
            stream,
            b"\nendstream".to_vec(),
        ]
        .concat(),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>".to_vec(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    for _ in 0..padding / 64 {
        out.extend_from_slice(&[b'%'; 63]);
        out.push(b'\n');
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn eml(subject: &str, from: &str, to: &str, sent: Instant, body: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {sent}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

struct Demo {
    rng: Rng,
    company: &'static Company,
    language: Language,
    shares: PathBuf,
    staff: Vec<Agent>,
    matters: Vec<(Matter, &'static MatterSpec)>,
    counters: Vec<u32>,
}

/// Write the demonstration root in `language` under `out`, which is created
/// and must be empty: `root/` and `shares/`. `windows_shares` is the path
/// `settings.toml` gives the share root on Windows where it is not where it
/// is written, for a root that is zipped and unpacked on another machine.
///
/// # Errors
///
/// A sentence saying what could not be done.
pub fn run(
    out: &Path,
    language: Language,
    seed: u64,
    windows_shares: Option<&str>,
) -> Result<(), String> {
    let company = match language {
        Language::En => &EN,
        Language::De => &DE,
    };
    let out = std::path::absolute(out).map_err(err)?;
    let root = out.join("root");
    let shares = out.join("shares");
    for dir in ["schedule", "register", "holds", "aggregations", "fixity"] {
        std::fs::create_dir_all(root.join(dir)).map_err(err)?;
    }
    std::fs::create_dir_all(&shares).map_err(err)?;
    let here = shares.to_string_lossy().into_owned();
    let windows = windows_shares.unwrap_or(&here);
    std::fs::write(
        root.join("settings.toml"),
        format!(
            "organization = \"{}\"\nfiscal_year_start_month = {}\n\n[roots.{}]\nwindows = '{windows}'\nmacos = '{here}'\nlinux = '{here}'\n",
            company.organization, company.fiscal_year_start_month, company.share
        ),
    )
    .map_err(err)?;

    let agent = |local: &str, n: usize| Agent {
        email: format!("{local}@{}", company.domain),
        id: format!("dir-{n:04}"),
    };
    let staff: Vec<Agent> = company
        .staff
        .iter()
        .enumerate()
        .map(|(i, l)| agent(l, i + 1))
        .collect();
    let records_manager = agent(company.records_manager, 90);
    let counsel = agent(company.counsel, 91);

    let store = Store::open(root.join("schedule"));
    let version = store
        .write_version(
            &schedule_csv(company.series)?,
            instant(Date::new(2026, 1, 12).unwrap(), 9, 0, 0),
            &records_manager,
            company.schedule_source,
            TOOL,
        )
        .map_err(err)?;
    store.set_current(&version).map_err(err)?;

    let mut demo = Demo {
        rng: Rng::new(seed),
        company,
        language,
        shares,
        staff,
        matters: Vec::new(),
        counters: vec![0; company.kinds.len()],
    };
    demo.write_matters(&root, &counsel)?;
    let contracts = demo.write_aggregations(&root)?;
    demo.write_contracts(&contracts)?;
    for (k, kind) in company.kinds.iter().enumerate() {
        if kind.annual {
            for year in kind.years.0..=kind.years.1 {
                demo.record(k, Some(year))?;
            }
        }
    }
    let weighted: Vec<usize> = (0..company.kinds.len())
        .filter(|&k| !company.kinds[k].annual)
        .collect();
    let total: u64 = weighted
        .iter()
        .map(|&k| u64::from(company.kinds[k].weight))
        .sum();
    for _ in 0..total * 3 / 2 {
        let mut at = demo.rng.below(total);
        let mut chosen = weighted[0];
        for &k in &weighted {
            let w = u64::from(company.kinds[k].weight);
            if at < w {
                chosen = k;
                break;
            }
            at -= w;
        }
        demo.record(chosen, None)?;
    }
    println!(
        "wrote the {} demonstration root under {}",
        company.organization,
        out.display()
    );
    Ok(())
}

fn schedule_csv(series: &[SeriesRow]) -> Result<Vec<u8>, String> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record([
        "GRS ID",
        "Record Title",
        "Classification (General)",
        "Legal Citation",
        "Disposition",
        "Retention (Years)",
        "Retention Type",
        "Event Type (General)",
        "Longer Retention Authorized?",
        "x_disposal_action",
        "x_cutoff",
        "x_period_kind",
    ])
    .map_err(err)?;
    for row in series {
        w.write_record(row).map_err(err)?;
    }
    w.into_inner().map_err(err)
}

impl Demo {
    fn id(&mut self, at: Instant) -> Identifier {
        self.rng.uuid7(unix_ms(at))
    }

    fn write_matters(&mut self, root: &Path, counsel: &Agent) -> Result<(), String> {
        for spec in self.company.matters {
            let placed = date(spec.placed);
            let mut scope = format!("custodian.email = \"{}\"", self.staff[spec.custodian].email);
            if let Some(from) = spec.created_from {
                scope.push_str(&format!(" and created >= {}", date(from)));
            }
            if let Some(before) = spec.created_before {
                scope.push_str(&format!(" and created < {}", date(before)));
            }
            let matter = Matter {
                id: self.id(instant(placed, 9, 0, 0)),
                title: spec.title.into(),
                kind: spec.kind,
                mandate: spec.mandate.into(),
                approved_by: counsel.clone(),
                placed,
                end_date: spec.end.map(date),
                scope,
                released: spec.released.map(date),
                tool: TOOL.into(),
            };
            std::fs::write(
                root.join("holds").join(format!("{}.slpc", matter.id)),
                matter.build(),
            )
            .map_err(err)?;
            self.matters.push((matter, spec));
        }
        Ok(())
    }

    /// The contracts aggregation and one aggregation per contract under it.
    fn write_aggregations(&mut self, root: &Path) -> Result<Vec<Aggregation>, String> {
        let actor = self.staff[self.company.contract_custodian].clone();
        let created = Date::new(2016, 3, 14).unwrap();
        let parent = Aggregation {
            id: self.id(instant(created, 9, 0, 0)),
            title: self.company.contracts_title.into(),
            default_series: None,
            created,
            closed: None,
            parent: None,
            actor: actor.clone(),
            tool: TOOL.into(),
        };
        let mut all = Vec::new();
        for c in self.company.contracts {
            let started = date(c.started);
            all.push(Aggregation {
                id: self.id(instant(started, 9, 0, 0)),
                title: c.title.into(),
                default_series: Some(self.company.contract_series.into()),
                created: started,
                closed: c.closed.map(date),
                parent: Some((parent.id.clone(), parent.title.clone())),
                actor: actor.clone(),
                tool: TOOL.into(),
            });
        }
        all.insert(0, parent);
        for a in &all {
            std::fs::write(
                root.join("aggregations").join(format!("{}.slpc", a.id)),
                a.build(),
            )
            .map_err(err)?;
        }
        Ok(all.split_off(1))
    }

    fn write_contracts(&mut self, contracts: &[Aggregation]) -> Result<(), String> {
        let company = self.company;
        for a in contracts {
            let number = a.title.split([' ', ',']).nth(1).unwrap_or("0000");
            for (d, (stem, title)) in company.contract_documents.iter().enumerate() {
                let created = add_days(a.created, (d as i64) * 45);
                let name = format!("{}-{stem}.pdf", slug(number));
                let rel = format!("{}/{}/{name}.slpc", company.contract_folder, slug(number));
                let full_title = format!("{title}, {}", a.title);
                let mut record = self.base(company.contract_custodian, created, &name, &rel);
                record.content = pdf(
                    company.organization,
                    &full_title,
                    created,
                    company.footer,
                    self.padding(),
                );
                record.series = vec![SeriesEntry {
                    code: company.contract_series.into(),
                    trigger: a.closed,
                    snapshot: None,
                }];
                record
                    .relations
                    .push(("member-of", a.id.clone(), a.title.clone()));
                self.finish(record, &rel)?;
            }
        }
        Ok(())
    }

    fn padding(&mut self) -> usize {
        self.rng.range(9_000, 140_000) as usize
    }

    fn base(&mut self, person: usize, created: Date, name: &str, rel: &str) -> Record {
        let captured = instant(
            add_days(created, self.rng.range(0, 20)),
            self.rng.range(8, 17) as u8,
            self.rng.range(0, 59) as u8,
            0,
        );
        let agent = self.staff[person].clone();
        Record {
            id: self.id(captured),
            created,
            captured,
            content_name: name.to_owned(),
            mime_type: "application/pdf".into(),
            content: Vec::new(),
            essential: false,
            marking: None,
            creator: agent.clone(),
            custodian: agent,
            series: Vec::new(),
            holds: Vec::new(),
            relations: Vec::new(),
            components: Vec::new(),
            second_profile: false,
            location: Location {
                root: Some(self.company.share.into()),
                path: Some(rel.to_owned()),
                raw: self.shares.join(rel).to_string_lossy().into_owned(),
            },
            events: Vec::new(),
            tool: TOOL.into(),
        }
    }

    fn record(&mut self, k: usize, year: Option<i64>) -> Result<(), String> {
        let company = self.company;
        let kind = &company.kinds[k];
        self.counters[k] += 1;
        let n = self.counters[k];
        let year = year.unwrap_or_else(|| self.rng.range(kind.years.0, kind.years.1));
        let last_month = if year == 2026 { 9 } else { 12 };
        let month = self.rng.range(1, last_month) as u8;
        let created = if kind.annual {
            Date::new(year as u16, 12, self.rng.range(15, 28) as u8).unwrap()
        } else {
            Date::new(year as u16, month, self.rng.range(1, 28) as u8).unwrap()
        };
        let policies = match self.language {
            Language::En => POLICIES_EN,
            Language::De => POLICIES_DE,
        };
        let lanes = match self.language {
            Language::En => LANES_EN,
            Language::De => LANES_DE,
        };
        let party = if kind.series.starts_with("ADM-200") || kind.series.starts_with("ORG-") {
            policies[(n as usize - 1) % policies.len()].to_owned()
        } else if kind.folder == "operations" || kind.folder == "logistik" {
            self.rng.pick(lanes).to_string()
        } else {
            self.rng.pick(company.parties).to_string()
        };
        let name = capitalized(company.names[(n as usize - 1) % company.names.len()]);
        let subject = self.rng.pick(company.subjects).to_string();
        let number = format!("{:04}", n * 7 + 100);
        let fill = |pattern: &str| {
            pattern
                .replace("{y}", &year.to_string())
                .replace("{m}", &format!("{month:02}"))
                .replace("{month}", company.months[month as usize - 1])
                .replace("{n}", &number)
                .replace("{party}", &party)
                .replace("{name}", &name)
                .replace("{subject}", &subject)
        };
        let title = fill(kind.title);
        let stem = slug(&fill(kind.stem));
        let extension = if kind.email { "eml" } else { "pdf" };
        let file = format!("{stem}.{extension}");
        let rel = format!("{}/{year}/{file}.slpc", kind.folder);
        let person = *self.rng.pick(kind.staff);
        let mut record = self.base(person, created, &file, &rel);
        if kind.email {
            let to = format!("{}@{}", slug(&party), "customer.example");
            let sent = instant(
                created,
                self.rng.range(8, 17) as u8,
                self.rng.range(0, 59) as u8,
                0,
            );
            let body = match self.language {
                Language::En => "Thank you for your message. We confirm the arrangement as discussed.",
                Language::De => "Vielen Dank für Ihre Nachricht. Wir bestätigen die Vereinbarung wie besprochen.",
            };
            record.content = eml(&title, &record.custodian.email, &to, sent, body);
            record.mime_type = "message/rfc822".into();
        } else {
            record.content = pdf(
                company.organization,
                &title,
                created,
                company.footer,
                self.padding(),
            );
        }
        if kind.marking {
            record.marking = Some(company.marking.into());
        }
        let trigger = kind.event.and_then(|(from, to, share)| {
            self.rng.chance(share).then(|| {
                let t = add_days(created, self.rng.range(from, to));
                if t > Date::new(2026, 9, 30).unwrap() {
                    Date::new(2026, 9, 30).unwrap()
                } else {
                    t
                }
            })
        });
        record.series = vec![SeriesEntry {
            code: kind.series.into(),
            trigger,
            snapshot: None,
        }];
        self.finish(record, &rel)
    }

    /// Apply every active matter whose scope the record falls in, then
    /// write it.
    fn finish(&mut self, mut record: Record, rel: &str) -> Result<(), String> {
        for (matter, spec) in &self.matters {
            let in_scope = matter.released.is_none()
                && record.custodian.email == self.staff[spec.custodian].email
                && spec.created_from.is_none_or(|f| record.created >= date(f))
                && spec.created_before.is_none_or(|b| record.created < date(b));
            if in_scope {
                let applied = if matter.placed > record.created {
                    add_days(matter.placed, 1)
                } else {
                    record.created
                };
                record.holds.push((matter.id.clone(), matter.kind, applied));
            }
        }
        let bytes = record.build(&crate::build::Tamper::None);
        let path = self.shares.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
}
