use super::{WorkspaceCitation, WorkspacePanelRequest, WorkspaceToolStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Row {
    Prompt(String),
    Notice {
        text: String,
        failed: bool,
    },
    Answer {
        text: String,
        sources: Vec<WorkspaceCitation>,
    },
    Tool {
        title: String,
        target: String,
        status: Option<WorkspaceToolStatus>,
        detail: String,
    },
}

#[derive(Default)]
pub(super) struct Conversation {
    pub rows: Vec<Row>,
    pub running: bool,
    pub unconfigured: bool,
}

impl Conversation {
    pub fn cite(&self, id: u32) -> Option<WorkspacePanelRequest> {
        self.rows
            .iter()
            .any(|row| match row {
                Row::Answer { sources, .. } => sources.iter().any(|source| source.id == id),
                Row::Prompt(_) | Row::Notice { .. } | Row::Tool { .. } => false,
            })
            .then_some(WorkspacePanelRequest::Cite(id))
    }

    pub fn submit(&self, text: &str) -> Option<String> {
        let text = text.trim();
        (!self.running && !self.unconfigured && !text.is_empty()).then(|| text.to_owned())
    }

    pub fn start_tool(&mut self, title: &str, target: &str) -> usize {
        let index = self.rows.len();
        self.rows.push(Row::Tool {
            title: title.to_owned(),
            target: target.to_owned(),
            status: None,
            detail: String::new(),
        });
        index
    }

    pub fn finish_tool(&mut self, row: usize, status: WorkspaceToolStatus, detail: &str) {
        if let Some(Row::Tool {
            status: current,
            detail: result,
            ..
        }) = self.rows.get_mut(row)
        {
            *current = Some(status);
            detail.clone_into(result);
        }
    }
}

#[cfg(test)]
mod tests;
