use std::fmt::Write;

use quickie_application::{AgentLink, AgentLinks, AgentPage};
use url::Url;

const LIMIT: usize = 24;

#[derive(Clone)]
pub(super) struct Observed {
    pub(super) page: AgentPage,
    pub(super) revision: u64,
    pub(super) link: AgentLink,
    document: Url,
}

impl Observed {
    pub(super) fn is_in(&self, snapshot: &AgentLinks) -> bool {
        snapshot.document == self.document && snapshot.links.contains(&self.link)
    }
}

/// Link IDs remain local to the workspace, with only the latest snapshot eligible.
#[derive(Default)]
pub(in crate::browser::agent::workspace) struct Observations {
    serial: u32,
    links: Vec<(u32, Observed)>,
}

impl Observations {
    pub(in crate::browser::agent::workspace) fn clear(&mut self) {
        self.links.clear();
    }

    pub(super) fn get(&self, id: u32) -> Option<&Observed> {
        self.links
            .iter()
            .find_map(|(candidate, link)| (*candidate == id).then_some(link))
    }

    pub(super) fn label(&self, id: u32) -> String {
        self.get(id).map_or_else(
            || "Unobserved link".to_owned(),
            |link| link.link.label.clone(),
        )
    }

    pub(super) fn record(
        &mut self,
        snapshot: AgentLinks,
        page: AgentPage,
        revision: u64,
    ) -> (String, String) {
        self.clear();
        let mut listed = String::new();
        for link in snapshot
            .links
            .into_iter()
            .filter(|link| {
                matches!(link.url.scheme(), "http" | "https")
                    && link.url.username().is_empty()
                    && link.url.password().is_none()
                    && link.url.origin() == snapshot.document.origin()
            })
            .take(LIMIT)
        {
            let Some(serial) = self.serial.checked_add(1) else {
                break;
            };
            self.serial = serial;
            let label = link.label.split_whitespace().collect::<Vec<_>>().join(" ");
            let hint = if link.is_video() { " (video)" } else { "" };
            let _ = writeln!(listed, "{serial}: {label}{hint}");
            self.links.push((
                serial,
                Observed {
                    page,
                    revision,
                    link,
                    document: snapshot.document.clone(),
                },
            ));
        }
        if self.links.is_empty() {
            return (
                "No eligible links.".to_owned(),
                "No eligible links".to_owned(),
            );
        }
        let count = self.links.len();
        (listed, format!("{count} links found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_id_is_bound_to_the_latest_snapshot_and_destination() {
        let document = Url::parse("https://example.com/").unwrap();
        let video = AgentLink {
            url: document.join("/watch?v=abc").unwrap(),
            label: "First video".into(),
        };
        let mut observations = Observations::default();
        let snapshot = AgentLinks {
            document,
            links: vec![video],
        };
        let (listed, _) = observations.record(snapshot.clone(), AgentPage::Source, 3);
        assert_eq!(listed, "1: First video (video)\n");
        assert!(observations.get(1).unwrap().is_in(&snapshot));

        let mut changed = snapshot;
        changed.links[0].url.set_query(Some("v=other"));
        assert!(!observations.get(1).unwrap().is_in(&changed));
        observations.record(changed, AgentPage::Source, 4);
        assert!(observations.get(1).is_none());
        assert_eq!(observations.get(2).unwrap().revision, 4);
    }

    #[test]
    fn a_foreign_link_never_gets_a_reusable_id() {
        let snapshot = AgentLinks {
            document: Url::parse("https://example.com/").unwrap(),
            links: vec![AgentLink {
                url: Url::parse("https://example.net/").unwrap(),
                label: "External".into(),
            }],
        };
        let mut observations = Observations::default();
        let (listed, _) = observations.record(snapshot, AgentPage::Source, 1);
        assert_eq!(listed, "No eligible links.");
        assert!(observations.get(1).is_none());
    }
}
