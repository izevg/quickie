//! The agent settings: saving goes through the agent use cases (the endpoint to the store, the
//! key to the Keychain), and listing models and looking for a local LLM run on a worker thread,
//! each only on its click. The key typed is read once, on save or on listing models, and never
//! shown, logged, or kept by the shell.

use quickie_application::{LlmPort, LocalFound, detect_local, list_models};
use quickie_domain_agents::{ApiKey, BaseUrl, ModelId};
use quickie_engine_webkit::UrlSessionHttp;
use quickie_llm::OpenAiClient;
use quickie_ui_screens::Pane;
use quickie_ui_widgets::{SettingsForm, SettingsRequest, SettingsSummary, Tone, agent_wording};

use super::super::Browser;

/// What a worker found, in words the form shows.
type Listed = Result<Vec<String>, String>;

impl Browser {
    /// "Show Agent Settings": the Settings window on its Agent pane.
    pub(in crate::browser) fn open_agent_settings(&self) {
        self.open_settings(Some(Pane::Agent));
    }

    /// The form shows the endpoint as saved.
    pub(in crate::browser) fn load_agent_settings(&self) {
        let summary =
            self.agent
                .borrow()
                .endpoint()
                .map_or_else(SettingsSummary::default, |endpoint| SettingsSummary {
                    base: endpoint.base().as_str().to_owned(),
                    model: endpoint.model().as_str().to_owned(),
                    has_key: endpoint.key().is_some(),
                });
        self.agent_ui.settings.load(&summary);
    }

    /// What the form asked.
    pub(super) fn settings_request(&self, request: SettingsRequest) {
        match request {
            SettingsRequest::Save(form) => self.save_settings(&form),
            SettingsRequest::ListModels(form) => self.list_models(&form),
            SettingsRequest::DetectLocal => self.detect_local(),
            SettingsRequest::RemoveKey => self.remove_key(),
        }
    }

    /// Checks the whole form first, so a bad field changes nothing; then the endpoint, then the
    /// key.
    fn save_settings(&self, form: &SettingsForm) {
        let settings = &self.agent_ui.settings;
        let parsed = parse(form).and_then(|(base, model)| {
            let key = (!form.key.is_empty())
                .then(|| ApiKey::try_from(form.key.as_str()))
                .transpose()
                .map_err(|error| format!("API key: {error}."))?;
            Ok((base, model, key))
        });
        let (base, model, key) = match parsed {
            Ok(parsed) => parsed,
            Err(problem) => return settings.set_status(&problem, Tone::Problem),
        };
        let mut agent = self.agent.borrow_mut();
        let saved = agent.set_endpoint(base, model).and_then(|()| match &key {
            Some(key) => agent.set_key(key),
            None => Ok(()),
        });
        let endpoint = agent.endpoint().cloned();
        drop(agent);
        if let Err(error) = saved {
            return settings.set_status(&format!("Not saved: {error}."), Tone::Problem);
        }
        let has_key = endpoint
            .as_ref()
            .is_some_and(|endpoint| endpoint.key().is_some());
        settings.key_saved(has_key);
        let chip = endpoint
            .as_ref()
            .map_or_else(String::new, agent_wording::model_chip);
        let key_note = if key.is_some() {
            " The key is in the Keychain."
        } else {
            ""
        };
        settings.set_status(
            &format!("Saved: the agent asks {chip}.{key_note}"),
            Tone::Success,
        );
    }

    /// Asks the endpoint in the form for its models: with the key typed, else with the saved
    /// key when the form keeps the saved endpoint's origin, else with none.
    fn list_models(&self, form: &SettingsForm) {
        let settings = &self.agent_ui.settings;
        let base = match BaseUrl::try_from(form.base.as_str()) {
            Ok(base) => base,
            Err(error) => {
                return settings.set_status(&format!("Endpoint: {error}."), Tone::Problem);
            }
        };
        let typed = if form.key.is_empty() {
            None
        } else {
            match ApiKey::try_from(form.key.as_str()) {
                Ok(key) => Some(key),
                Err(error) => {
                    return settings.set_status(&format!("API key: {error}."), Tone::Problem);
                }
            }
        };
        let saved = self
            .agent
            .borrow()
            .endpoint()
            .map(|saved| saved.moved(base.clone(), saved.model().clone()));
        let secrets = self.agent_ui.secrets.clone();
        let host = base.as_url().host_str().unwrap_or_default().to_owned();
        let work = move || -> Listed {
            let llm = OpenAiClient::new(UrlSessionHttp::new());
            let models = match (typed, saved) {
                (Some(key), _) => llm
                    .models(&base, Some(&key))
                    .map_err(|error| agent_wording::llm_error(&error)),
                (None, Some(endpoint)) => list_models(&llm, &secrets, &endpoint)
                    .map_err(|error| agent_wording::ask_error(&error)),
                (None, None) => llm
                    .models(&base, None)
                    .map_err(|error| agent_wording::llm_error(&error)),
            };
            models
                .map(|models| {
                    models
                        .iter()
                        .map(|model| model.as_str().to_owned())
                        .collect()
                })
                .map_err(|problem| format!("{problem} ({host})"))
        };
        settings.set_busy(true);
        settings.set_status("Asking the endpoint for its models…", Tone::Neutral);
        if !self.off_main("quickie-agent-models", work, Self::models_listed) {
            settings.set_busy(false);
            settings.set_status("Couldn't start asking for models.", Tone::Problem);
        }
    }

    fn models_listed(&self, listed: Listed) {
        let settings = &self.agent_ui.settings;
        settings.set_busy(false);
        match listed {
            Ok(models) if models.is_empty() => {
                settings.set_status("The endpoint lists no models.", Tone::Problem);
            }
            Ok(models) => {
                settings.set_models(&models);
                settings.set_status(&models_found(models.len()), Tone::Success);
            }
            Err(problem) => settings.set_status(&problem, Tone::Problem),
        }
    }

    /// Looks for Ollama and LM Studio on this Mac; the first found fills the form.
    fn detect_local(&self) {
        let settings = &self.agent_ui.settings;
        settings.set_busy(true);
        settings.set_status("Looking for a local LLM on this Mac…", Tone::Neutral);
        let work = || detect_local(&OpenAiClient::new(UrlSessionHttp::new()));
        if !self.off_main("quickie-agent-detect", work, Self::local_found) {
            settings.set_busy(false);
            settings.set_status("Couldn't start looking.", Tone::Problem);
        }
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "a worker's result handed over whole, as `off_main` hands every result"
    )]
    fn local_found(&self, found: Vec<LocalFound>) {
        let settings = &self.agent_ui.settings;
        settings.set_busy(false);
        let Some(first) = found.first() else {
            return settings.set_status(
                "No local LLM answered: Ollama (port 11434) and LM Studio (port 1234) aren't \
                 running on this Mac.",
                Tone::Problem,
            );
        };
        settings.set_base(first.base.as_str());
        let models: Vec<String> = first
            .models
            .iter()
            .map(|model| model.as_str().to_owned())
            .collect();
        settings.set_models(&models);
        settings.set_status(
            &format!(
                "Found {} with {}. Save to use it.",
                first.server.name(),
                models_found(models.len()).to_lowercase()
            ),
            Tone::Success,
        );
    }

    fn remove_key(&self) {
        let settings = &self.agent_ui.settings;
        let removed = self.agent.borrow_mut().remove_key();
        match removed {
            Ok(()) => {
                settings.key_saved(false);
                settings.set_status("The key is gone from the Keychain.", Tone::Neutral);
            }
            Err(error) => settings.set_status(&format!("Not removed: {error}."), Tone::Problem),
        }
    }
}

/// The endpoint and the model of the form, or what is wrong with them.
fn parse(form: &SettingsForm) -> Result<(BaseUrl, ModelId), String> {
    let base =
        BaseUrl::try_from(form.base.as_str()).map_err(|error| format!("Endpoint: {error}."))?;
    let model =
        ModelId::try_from(form.model.as_str()).map_err(|error| format!("Model: {error}."))?;
    Ok((base, model))
}

/// `3 models`, `1 model`.
fn models_found(count: usize) -> String {
    if count == 1 {
        "1 model".to_owned()
    } else {
        format!("{count} models")
    }
}

#[cfg(test)]
mod tests {
    use quickie_ui_widgets::SettingsForm;

    use super::{models_found, parse};

    fn form(base: &str, model: &str) -> SettingsForm {
        SettingsForm {
            base: base.to_owned(),
            model: model.to_owned(),
            key: String::new(),
        }
    }

    #[test]
    fn a_form_needs_an_endpoint_and_a_model() {
        assert!(parse(&form("http://127.0.0.1:11434/v1", "qwen3")).is_ok());
        assert_eq!(
            parse(&form("http://example.com/v1", "m")).unwrap_err(),
            "Endpoint: plain http is only for this machine, not example.com; use https."
        );
        assert_eq!(
            parse(&form("https://example.com/v1", "")).unwrap_err(),
            "Model: a model id is not empty."
        );
    }

    #[test]
    fn counts_read_as_words() {
        assert_eq!(models_found(1), "1 model");
        assert_eq!(models_found(3), "3 models");
    }
}
