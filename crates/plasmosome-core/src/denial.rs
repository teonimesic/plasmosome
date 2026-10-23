use crate::manifest::{is_proxy_host, toml_string};
use plasmosome_backend::{CellId, PluginId, WallMs};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{collections::VecDeque, fmt, marker::PhantomData};
use uuid::Uuid;

/// The session log kind for the payload returned by [`log_event`].
pub const DENIAL_EVENT: &str = "denial";

/// A refused host or port. Use the constructors to check declaration syntax;
/// callers constructing variants directly must pass a valid host and a nonzero port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// A host no grant holds.
    Host { host: String },
    /// A port no grant holds at this host.
    Port { host: String, port: u16 },
}

impl Reach {
    /// Returns a host reach only for a host the manifest accepts, without normalization.
    pub fn host(host: &str) -> Option<Reach> {
        is_proxy_host(host).then(|| Reach::Host { host: host.into() })
    }

    /// Returns a port reach only for a manifest host and a port other than zero.
    pub fn port(host: &str, port: u16) -> Option<Reach> {
        (is_proxy_host(host) && port != 0).then(|| Reach::Port {
            host: host.into(),
            port,
        })
    }

    /// Returns only the denied reach and the bare declaration entry that would allow it.
    pub fn report(&self) -> Report {
        match self {
            Reach::Host { host } => Report {
                denied: host.clone(),
                field: "network.hosts",
                fix: toml_string(host),
            },
            Reach::Port { host, port } => Report {
                denied: format!("{host}:{port}"),
                field: "network.ports",
                fix: port.to_string(),
            },
        }
    }
}

/// A missing declaration in the author's vocabulary; `fix` is an entry, not a replacement list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The host, or host and port, that the workload chose.
    pub denied: String,
    /// The declaration list to which the repair belongs.
    pub field: &'static str,
    /// The bare TOML entry to add to that list.
    pub fix: String,
}

fn object_only<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct Object<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> de::Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a denial record object")
        }
        fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(PhantomData))
}

fn present_plasmid<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PluginId>, D::Error> {
    PluginId::deserialize(deserializer).map(Some)
}

fn zero(value: &u64) -> bool {
    *value == 0
}

macro_rules! wire_record {
    ($(#[$doc:meta])* $name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name { $($(#[$attr])* pub $field: $ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Fields { $($(#[$attr])* $field: $ty),* }
                let fields: Fields = object_only(deserializer)?;
                Ok(Self { $($field: fields.$field),* })
            }
        }
    };
}

wire_record! {
    /// One distinct denial and its saturating count. Decoding requires exactly these four fields.
    DenialEntry {
        /// The refused reach in the declaration's vocabulary.
        denied: String,
        /// The declaration list the repair names.
        field: String,
        /// The bare entry to add.
        fix: String,
        /// All observations of this denial, saturating at u64::MAX.
        count: u64,
    }
}
wire_record! {
    /// One plasmid's entries in first-seen order; unnamed denials have no such record.
    PlasmidDenials {
        /// The caller-attributed owner of these denials.
        plasmid: PluginId,
        /// Distinct denials in first-seen order.
        denials: Vec<DenialEntry>,
    }
}
wire_record! {
    /// A waiting wire line. `seq` belongs to the book, not the session log.
    /// An absent plasmid and zero unlogged count are omitted, never sent as null.
    DenialLine {
        /// Monotonic within one book, beginning at one.
        seq: u64,
        /// Absent when no single plasmid owns the reach.
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_plasmid")]
        plasmid: Option<PluginId>,
        /// The refused reach.
        denied: String,
        /// The declaration list the repair names.
        field: String,
        /// The bare entry to add.
        fix: String,
        /// Observations of this denial through this line.
        count: u64,
        /// Boundary time in Unix milliseconds, supplied by the caller.
        at_ms: u64,
        /// Denials suppressed since the preceding issued line.
        #[serde(default, skip_serializing_if = "zero")]
        unlogged: u64,
    }
}
wire_record! {
    /// A bounded page of waiting lines and all named entries. Acknowledge only this book's lines.
    DenialPage {
        /// The cell the caller requested.
        cell: CellId,
        /// The identity that scopes acknowledgments.
        book: Uuid,
        /// Oldest unacknowledged lines, bounded by the page limit.
        lines: Vec<DenialLine>,
        /// All named entries grouped by first-seen plasmid.
        entries: Vec<PlasmidDenials>,
    }
}

/// Bounds for one book. Production callers use [`Self::DEFAULT`]; tests may inject smaller bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DenialLimits {
    /// Distinct entries for each named plasmid.
    pub per_plasmid: usize,
    /// Distinct named entries across the cell.
    pub named: usize,
    /// Distinct unnamed entries in their separate pool.
    pub unnamed: usize,
    /// Issued lines for each retained denial entry.
    pub lines_per_denial: u32,
    /// Issued lines over the book's entire life.
    pub lines_per_cell: u32,
    /// Lines waiting for acknowledgment.
    pub waiting: usize,
    /// Lines returned by one page.
    pub page: usize,
}

impl DenialLimits {
    /// Spec 011's entry and line bounds, with spec 001's 64-line reply bound.
    pub const DEFAULT: DenialLimits = DenialLimits {
        per_plasmid: 16,
        named: 64,
        unnamed: 32,
        lines_per_denial: 64,
        lines_per_cell: 4_096,
        waiting: 256,
        page: 64,
    };
}

struct Entry {
    plasmid: Option<PluginId>,
    reach: Reach,
    count: u64,
    lines: u32,
}

/// One cell's denial state, without IO or a clock. Attribution is the caller's responsibility.
/// Keep one book for the membrane's life and pass the boundary's time to each record.
pub struct DenialBook {
    id: Uuid,
    limits: DenialLimits,
    entries: Vec<Entry>,
    waiting: VecDeque<DenialLine>,
    issued: u32,
    last_seq: u64,
    unlogged: u64,
}

impl DenialBook {
    /// Creates an empty book. The membrane supplies a fresh UUID at start.
    pub fn new(book: Uuid, limits: DenialLimits) -> DenialBook {
        Self {
            id: book,
            limits,
            entries: Vec::new(),
            waiting: VecDeque::new(),
            issued: 0,
            last_seq: 0,
            unlogged: 0,
        }
    }

    /// Returns the identity that scopes acknowledgments to this membrane's life.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Counts an enforced denial and queues a line if room remains. Pass a plasmid only
    /// when the boundary knows it owns the reach under spec 011's attribution rule.
    pub fn record(&mut self, plasmid: Option<PluginId>, reach: Reach, at: WallMs) {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.plasmid == plasmid && entry.reach == reach);
        let index = match index {
            Some(index) => index,
            None => {
                let same = self
                    .entries
                    .iter()
                    .filter(|entry| entry.plasmid == plasmid)
                    .count();
                let named = self
                    .entries
                    .iter()
                    .filter(|entry| entry.plasmid.is_some())
                    .count();
                let room = if plasmid.is_some() {
                    same < self.limits.per_plasmid && named < self.limits.named
                } else {
                    same < self.limits.unnamed
                };
                if !room {
                    self.unlogged = self.unlogged.saturating_add(1);
                    return;
                }
                self.entries.push(Entry {
                    plasmid: plasmid.clone(),
                    reach,
                    count: 0,
                    lines: 0,
                });
                self.entries.len() - 1
            }
        };
        let entry = &mut self.entries[index];
        entry.count = entry.count.saturating_add(1);
        if entry.lines >= self.limits.lines_per_denial
            || self.issued >= self.limits.lines_per_cell
            || self.waiting.len() >= self.limits.waiting
            || self.last_seq == u64::MAX
        {
            self.unlogged = self.unlogged.saturating_add(1);
            return;
        }
        let report = entry.reach.report();
        self.last_seq += 1;
        entry.lines += 1;
        self.issued += 1;
        self.waiting.push_back(DenialLine {
            seq: self.last_seq,
            plasmid,
            denied: report.denied,
            field: report.field.into(),
            fix: report.fix,
            count: entry.count,
            at_ms: at.unix_ms(),
            unlogged: self.unlogged,
        });
        self.unlogged = 0;
    }

    /// Acknowledges `after` only for this book's id, then returns the oldest waiting lines
    /// without consuming them. No book or another book answers from the first waiting line.
    pub fn page(&mut self, cell: &CellId, book: Option<Uuid>, after: u64) -> DenialPage {
        if book == Some(self.id) {
            self.waiting.retain(|line| line.seq > after);
        }
        let mut entries: Vec<PlasmidDenials> = Vec::new();
        for entry in &self.entries {
            let Some(plasmid) = &entry.plasmid else {
                continue;
            };
            let group = match entries.iter().position(|group| group.plasmid == *plasmid) {
                Some(index) => index,
                None => {
                    entries.push(PlasmidDenials {
                        plasmid: plasmid.clone(),
                        denials: Vec::new(),
                    });
                    entries.len() - 1
                }
            };
            let report = entry.reach.report();
            entries[group].denials.push(DenialEntry {
                denied: report.denied,
                field: report.field.into(),
                fix: report.fix,
                count: entry.count,
            });
        }
        DenialPage {
            cell: cell.clone(),
            book: self.id,
            lines: self
                .waiting
                .iter()
                .take(self.limits.page)
                .cloned()
                .collect(),
            entries,
        }
    }

    /// Frees this plasmid's entry room, preserving its historical waiting lines and cell budget.
    pub fn forget(&mut self, plasmid: &PluginId) {
        self.entries
            .retain(|entry| entry.plasmid.as_ref() != Some(plasmid));
    }
}

/// Returns the payload to append under [`DENIAL_EVENT`]. The session log adds kind, time
/// and its own sequence; the wire sequence is carried here as `membrane_seq`.
pub fn log_event(cell: &CellId, line: &DenialLine) -> serde_json::Value {
    let mut event = serde_json::json!({ "cell": cell, "denied": line.denied, "field": line.field,
        "fix": line.fix, "count": line.count, "at_ms": line.at_ms, "membrane_seq": line.seq });
    if let Some(plasmid) = &line.plasmid {
        event["plasmid"] = serde_json::json!(plasmid);
    }
    if line.unlogged != 0 {
        event["unlogged"] = serde_json::json!(line.unlogged);
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn book() -> DenialBook {
        DenialBook::new(Uuid::from_u128(1), DenialLimits::DEFAULT)
    }
    fn host(n: usize) -> Reach {
        Reach::host(&format!("h{n}.example.test")).unwrap()
    }
    fn record(book: &mut DenialBook, plasmid: Option<&str>, reach: Reach) {
        book.record(
            plasmid.map(PluginId::from),
            reach,
            WallMs::from_unix_ms(123),
        );
    }
    fn page(book: &mut DenialBook) -> DenialPage {
        book.page(&CellId::from("cell-1"), None, 0)
    }
    fn ack(book: &mut DenialBook, after: u64) -> DenialPage {
        book.page(&CellId::from("cell-1"), Some(book.id()), after)
    }

    #[test]
    fn a_host_reach_reports_the_bare_host_in_the_hosts_field() {
        assert_eq!(
            Reach::host("api.example.com").unwrap().report(),
            Report {
                denied: "api.example.com".into(),
                field: "network.hosts",
                fix: "\"api.example.com\"".into(),
            }
        );
    }

    #[test]
    fn a_port_reach_reports_host_and_port_and_fixes_the_port_alone() {
        assert_eq!(
            Reach::port("api.github.com", 8080).unwrap().report(),
            Report {
                denied: "api.github.com:8080".into(),
                field: "network.ports",
                fix: "8080".into(),
            }
        );
        assert!(Reach::port("api.github.com", u16::MAX).is_some());
    }

    #[test]
    fn only_a_host_a_declaration_could_hold_is_a_reach() {
        let label63 = "a".repeat(63);
        let name253 = format!("{label63}.{label63}.{label63}.{}", "a".repeat(61));
        let refused = [
            "API.example.com",
            "",
            "a..b",
            "-a.example",
            "a-.example",
            "a.example.",
            "192.0.2.1",
            "0x7f000001",
            "api_x.example",
            "api.123",
            "a.0xAF",
            " a.example",
            "é.example",
        ];
        for host in refused
            .iter()
            .map(|s| s.to_string())
            .chain(["a".repeat(64), format!("{name253}a")])
        {
            assert!(!is_proxy_host(&host), "{host}");
            assert_eq!(Reach::host(&host), None, "{host}");
            assert_eq!(Reach::port(&host, 443), None, "{host}");
        }
        for host in [
            "api.github.com",
            "xn--bcher-kva.example",
            &label63,
            &name253,
        ] {
            assert!(is_proxy_host(host));
            assert_eq!(
                Reach::host(host).unwrap(),
                Reach::Host { host: host.into() }
            );
            assert!(Reach::port(host, 1).is_some());
            assert_eq!(Reach::port(host, 0), None);
        }
    }

    #[test]
    fn the_report_names_nothing_granted() {
        let declared = crate::manifest::PlasmidManifest::parse(
            r#"
id = "p"
description = "Test a denial."
[network]
hosts = ["a.example.test", "b.example.test"]
ports = [443, 8443]
"#,
        )
        .unwrap();
        assert_eq!(declared.id, "p");
        for (reach, absent) in [
            (
                Reach::host("c.example.test").unwrap(),
                vec!["a.example.test", "b.example.test", "443", "8443"],
            ),
            (
                Reach::port("a.example.test", 8080).unwrap(),
                vec!["b.example.test", "443", "8443"],
            ),
        ] {
            let mut book = book();
            record(&mut book, Some("p"), reach);
            let page = page(&mut book);
            for value in [
                serde_json::to_value(&page.entries[0].denials[0]).unwrap(),
                log_event(&page.cell, &page.lines[0]),
            ] {
                let text = value.to_string();
                for granted in &absent {
                    assert!(!text.contains(granted), "{text}");
                }
            }
        }
    }

    #[test]
    fn a_repeat_raises_count_and_issues_a_line_each_time() {
        let mut book = book();
        for n in 1..=3 {
            book.record(Some("p".into()), host(1), WallMs::from_unix_ms(n));
        }
        let page = page(&mut book);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].denials.len(), 1);
        assert_eq!(page.entries[0].denials[0].count, 3);
        assert_eq!(
            page.lines
                .iter()
                .map(|l| (l.seq, l.count, l.at_ms))
                .collect::<Vec<_>>(),
            vec![(1, 1, 1), (2, 2, 2), (3, 3, 3)]
        );
    }

    #[test]
    fn entries_are_grouped_by_plasmid_in_first_seen_order() {
        let mut book = book();
        record(&mut book, Some("z"), host(1));
        record(&mut book, Some("a"), host(2));
        record(&mut book, Some("z"), host(3));
        let entries = page(&mut book).entries;
        assert_eq!(
            entries
                .iter()
                .map(|e| e.plasmid.as_str())
                .collect::<Vec<_>>(),
            vec!["z", "a"]
        );
        assert_eq!(
            entries[0]
                .denials
                .iter()
                .map(|d| d.denied.as_str())
                .collect::<Vec<_>>(),
            vec!["h1.example.test", "h3.example.test"]
        );
        assert_eq!(entries[1].denials[0].denied, "h2.example.test");
    }

    #[test]
    fn an_unnamed_denial_issues_a_line_and_no_entry() {
        let mut book = book();
        record(&mut book, None, host(1));
        let page = page(&mut book);
        assert!(page.entries.is_empty());
        assert_eq!(page.lines.len(), 1);
        assert_eq!(page.lines[0].plasmid, None);
        assert!(
            serde_json::to_value(&page.lines[0])
                .unwrap()
                .get("plasmid")
                .is_none()
        );
    }

    #[test]
    fn a_plasmid_holds_at_most_sixteen_entries() {
        let mut book = book();
        for n in 0..17 {
            record(&mut book, Some("p"), host(n));
        }
        assert_eq!(page(&mut book).entries[0].denials.len(), 16);
        assert_eq!(page(&mut book).lines.len(), 16);
        record(&mut book, Some("p"), host(0));
        record(&mut book, Some("p"), host(0));
        let page = page(&mut book);
        assert_eq!(page.lines[16].unlogged, 1);
        assert_eq!(page.lines[17].unlogged, 0);
        assert_eq!(page.lines[17].count, 3);
    }

    #[test]
    fn named_entries_stop_at_sixty_four_across_plasmids() {
        let mut book = book();
        for p in 0..5 {
            for n in 0..16 {
                record(&mut book, Some(&format!("p{p}")), host(n));
            }
        }
        let page = page(&mut book);
        assert_eq!(page.entries.len(), 4);
        assert_eq!(
            page.entries.iter().map(|e| e.denials.len()).sum::<usize>(),
            64
        );
        assert_eq!(book.waiting.len(), 64);
        record(&mut book, Some("p0"), host(0));
        assert_eq!(book.waiting.back().unwrap().unlogged, 16);
    }

    #[test]
    fn unnamed_room_is_separate() {
        let mut book = book();
        for n in 0..33 {
            record(&mut book, None, host(n));
        }
        assert_eq!(book.entries.len(), 32);
        assert_eq!(book.waiting.len(), 32);
        for p in 0..4 {
            for n in 0..16 {
                record(&mut book, Some(&format!("p{p}")), host(n));
            }
        }
        assert_eq!(book.entries.len(), 96);
        assert_eq!(book.waiting[32].unlogged, 1);
        assert_eq!(page(&mut book).entries.len(), 4);
    }

    #[test]
    fn a_denial_issues_at_most_sixty_four_lines() {
        let mut book = book();
        for _ in 0..65 {
            record(&mut book, Some("p"), host(1));
        }
        assert_eq!(page(&mut book).lines.len(), 64);
        assert_eq!(page(&mut book).entries[0].denials[0].count, 65);
        record(&mut book, Some("p"), host(2));
        assert_eq!(book.waiting.back().unwrap().unlogged, 1);
    }

    #[test]
    fn a_cell_issues_at_most_its_line_budget() {
        let mut book = DenialBook::new(
            Uuid::nil(),
            DenialLimits {
                lines_per_cell: 5,
                ..DenialLimits::DEFAULT
            },
        );
        for n in 0..5 {
            record(&mut book, Some("p"), host(n));
        }
        assert_eq!(page(&mut book).lines.len(), 5);
        ack(&mut book, 5);
        book.forget(&"p".into());
        record(&mut book, Some("p"), host(6));
        assert!(page(&mut book).lines.is_empty());
        assert_eq!(page(&mut book).entries[0].denials[0].count, 1);
        assert_eq!(book.unlogged, 1);
    }

    #[test]
    fn at_most_the_waiting_limit_waits() {
        let mut book = DenialBook::new(
            Uuid::nil(),
            DenialLimits {
                waiting: 3,
                ..DenialLimits::DEFAULT
            },
        );
        for _ in 0..4 {
            record(&mut book, Some("p"), host(1));
        }
        assert_eq!(page(&mut book).lines.len(), 3);
        assert_eq!(page(&mut book).entries[0].denials[0].count, 4);
        assert!(ack(&mut book, 3).lines.is_empty());
        record(&mut book, Some("p"), host(1));
        let page = page(&mut book);
        assert_eq!(
            (
                page.lines[0].seq,
                page.lines[0].count,
                page.lines[0].unlogged
            ),
            (4, 5, 1)
        );
    }

    #[test]
    fn a_page_drops_only_what_after_acknowledges() {
        let mut book = DenialBook::new(
            Uuid::nil(),
            DenialLimits {
                page: 2,
                ..DenialLimits::DEFAULT
            },
        );
        for n in 0..3 {
            record(&mut book, Some("p"), host(n));
        }
        let first = ack(&mut book, 0);
        assert_eq!(
            first.lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(ack(&mut book, 0), first);
        assert_eq!(
            ack(&mut book, 2)
                .lines
                .iter()
                .map(|l| l.seq)
                .collect::<Vec<_>>(),
            vec![3]
        );
        assert!(ack(&mut book, u64::MAX).lines.is_empty());
        assert_eq!(page(&mut book).entries[0].denials.len(), 3);
    }

    #[test]
    fn an_after_from_another_book_drops_nothing() {
        let mut book = book();
        for n in 0..3 {
            record(&mut book, Some("p"), host(n));
        }
        for id in [None, Some(Uuid::from_u128(2))] {
            let page = book.page(&CellId::from("cell-7"), id, 5);
            assert_eq!(page.book, book.id());
            assert_eq!(page.cell, CellId::from("cell-7"));
            assert_eq!(
                page.lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
                vec![1, 2, 3]
            );
        }
        assert_eq!(book.waiting.len(), 3);
    }

    #[test]
    fn forget_frees_room_and_keeps_waiting_lines() {
        let mut book = book();
        for n in 0..16 {
            record(&mut book, Some("p"), host(n));
        }
        record(&mut book, Some("q"), host(1));
        book.forget(&"p".into());
        assert_eq!(page(&mut book).entries[0].plasmid, PluginId::from("q"));
        assert_eq!(page(&mut book).entries.len(), 1);
        assert_eq!(page(&mut book).lines.len(), 17);
        for n in 0..16 {
            record(&mut book, Some("p"), host(n));
        }
        let page = page(&mut book);
        assert_eq!(page.entries[1].denials.len(), 16);
        assert_eq!(page.entries[1].denials[0].count, 1);
        assert_eq!(page.lines.last().unwrap().seq, 33);
        assert_eq!(book.issued, 33);
    }

    /// Spec 011 Contract Bounds (467-472 at 30f8cbed) and spec 001 membrane.cell.denials (856-867).
    #[test]
    fn the_default_limits_are_the_specs() {
        assert_eq!(
            DenialLimits::DEFAULT,
            DenialLimits {
                per_plasmid: 16,
                named: 64,
                unnamed: 32,
                lines_per_denial: 64,
                lines_per_cell: 4096,
                waiting: 256,
                page: 64
            }
        );
    }

    fn strict<T: for<'de> Deserialize<'de> + Serialize + fmt::Debug + PartialEq>(value: T) {
        let encoded = serde_json::to_value(&value).unwrap();
        assert_eq!(serde_json::from_value::<T>(encoded.clone()).unwrap(), value);
        let object = encoded.as_object().unwrap();
        for key in object.keys() {
            let mut missing = encoded.clone();
            missing.as_object_mut().unwrap().remove(key);
            if !matches!(key.as_str(), "plasmid" | "unlogged") || object.contains_key("denials") {
                assert!(
                    serde_json::from_value::<T>(missing).is_err(),
                    "missing {key}"
                );
            }
            let mut unknown = encoded.clone();
            unknown["unknown"] = json!(1);
            assert!(serde_json::from_value::<T>(unknown).is_err());
            let duplicate = format!(
                "{{{},{}",
                serde_json::to_string(key).unwrap() + ":" + &object[key].to_string(),
                &encoded.to_string()[1..]
            );
            assert!(
                serde_json::from_str::<T>(&duplicate).is_err(),
                "{duplicate}"
            );
        }
        for bad in [json!(null), json!(1), json!("bad")] {
            assert!(serde_json::from_value::<T>(bad).is_err());
        }
    }

    #[test]
    fn wire_records_are_exact_and_strict() {
        let mut book = book();
        record(&mut book, None, host(1));
        record(&mut book, Some("p"), host(2));
        let mut page = page(&mut book);
        let entry = page.entries[0].denials[0].clone();
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            json!({"denied":"h2.example.test",
            "field":"network.hosts", "fix":"\"h2.example.test\"", "count":1})
        );
        assert_eq!(
            serde_json::to_value(&page.lines[0]).unwrap(),
            json!({"seq":1,"denied":"h1.example.test",
            "field":"network.hosts","fix":"\"h1.example.test\"","count":1,"at_ms":123})
        );
        assert_eq!(
            serde_json::to_value(&page.entries[0]).unwrap(),
            json!({"plasmid":"p","denials":[entry.clone()]})
        );
        assert_eq!(
            serde_json::to_value(&page).unwrap(),
            json!({"cell":"cell-1",
            "book":"00000000-0000-0000-0000-000000000001", "lines":page.lines,"entries":page.entries})
        );
        strict(entry);
        strict(page.entries[0].clone());
        strict(page.lines[0].clone());
        page.lines[1].unlogged = 7;
        strict(page.lines[1].clone());
        strict(page.clone());
        assert!(
            serde_json::from_value::<DenialEntry>(json!([
                "h2.example.test",
                "network.hosts",
                "\"h2.example.test\"",
                1
            ]))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PlasmidDenials>(json!(["p", page.entries[0].denials]))
                .is_err()
        );
        assert!(
            serde_json::from_value::<DenialLine>(json!([
                2,
                "p",
                "h2.example.test",
                "network.hosts",
                "\"h2.example.test\"",
                1,
                123,
                7
            ]))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DenialPage>(json!([
                page.cell,
                page.book,
                page.lines,
                page.entries
            ]))
            .is_err()
        );
        let mut invalid = serde_json::to_value(&page.lines[0]).unwrap();
        invalid["plasmid"] = Value::Null;
        assert!(serde_json::from_value::<DenialLine>(invalid).is_err());
        let encoded = serde_json::to_string(&page).unwrap();
        let repeated = encoded.replacen("\"count\":1", "\"count\":1,\"count\":2", 1);
        assert!(serde_json::from_str::<DenialPage>(&repeated).is_err());
    }

    #[test]
    fn the_log_event_is_exact() {
        let mut book = book();
        record(&mut book, None, host(1));
        record(
            &mut book,
            Some("p"),
            Reach::port("api.github.com", 8080).unwrap(),
        );
        let mut page = page(&mut book);
        assert_eq!(DENIAL_EVENT, "denial");
        assert_eq!(
            log_event(&page.cell, &page.lines[0]),
            json!({"cell":"cell-1","denied":"h1.example.test",
            "field":"network.hosts","fix":"\"h1.example.test\"","count":1,"at_ms":123,"membrane_seq":1})
        );
        page.lines[1].unlogged = 2;
        assert_eq!(
            log_event(&page.cell, &page.lines[1]),
            json!({"cell":"cell-1","plasmid":"p",
            "denied":"api.github.com:8080","field":"network.ports","fix":"8080","count":1,
            "at_ms":123,"membrane_seq":2,"unlogged":2})
        );
    }

    #[test]
    fn count_saturates() {
        let mut book = book();
        record(&mut book, Some("p"), host(1));
        book.entries[0].count = u64::MAX - 1;
        for _ in 0..2 {
            record(&mut book, Some("p"), host(1));
        }
        assert_eq!(page(&mut book).entries[0].denials[0].count, u64::MAX);
        assert_eq!(book.waiting.back().unwrap().count, u64::MAX);
    }

    #[test]
    fn exhausted_sequences_and_unlogged_counts_never_wrap() {
        let mut book = book();
        record(&mut book, None, host(1));
        book.last_seq = u64::MAX - 1;
        record(&mut book, None, host(1));
        assert_eq!(book.waiting.back().unwrap().seq, u64::MAX);
        book.unlogged = u64::MAX;
        record(&mut book, None, host(1));
        assert_eq!(book.waiting.len(), 2);
        assert_eq!(book.entries[0].count, 3);
        assert_eq!(book.unlogged, u64::MAX);
        book.limits.unnamed = 0;
        record(&mut book, None, host(2));
        assert_eq!(book.unlogged, u64::MAX);
    }
}
