use serde_json::Value;

use super::{AutomationError, BrowsingContext};

#[derive(Default)]
pub(super) struct Reply {
    id: Option<u32>,
    result: Option<Result<Value, AutomationError>>,
}

impl Reply {
    pub(super) fn expect_id(&mut self, id: u32) {
        self.id = Some(id);
        self.result = None;
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn receive(&mut self, message: &str) {
        let Some(id) = self.id else { return };
        let Ok(mut response) = serde_json::from_str::<Value>(message) else {
            return;
        };
        if response["id"].as_u64() != Some(u64::from(id)) || self.result.is_some() {
            return;
        }
        self.result = Some(if let Some(error) = response.get("error") {
            if error["message"].as_str() == Some("Timeout") {
                Err(AutomationError::Timeout)
            } else {
                Err(AutomationError::CommandFailed)
            }
        } else if response["result"].is_object() {
            Ok(response["result"].take())
        } else {
            Err(AutomationError::InvalidReply)
        });
    }

    pub(super) fn take(&mut self) -> Option<Result<Value, AutomationError>> {
        self.result.take()
    }
}

pub(super) fn contexts(result: &Value) -> Result<Vec<BrowsingContext>, AutomationError> {
    result["contexts"]
        .as_array()
        .ok_or(AutomationError::InvalidReply)?
        .iter()
        .map(|context| {
            let handle = context["handle"]
                .as_str()
                .filter(|handle| !handle.is_empty())
                .ok_or(AutomationError::InvalidReply)?;
            let url = context["url"]
                .as_str()
                .ok_or(AutomationError::InvalidReply)?;
            Ok(BrowsingContext {
                handle: handle.to_owned(),
                url: url.to_owned(),
            })
        })
        .collect()
}
