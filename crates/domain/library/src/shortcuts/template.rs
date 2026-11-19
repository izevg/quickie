//! The URL a search shortcut opens, with a hole for the terms.

use url::{Url, form_urlencoded};

use crate::is_web;

/// Where the terms go in a template.
const PLACEHOLDER: &str = "%s";

/// A web URL with exactly one `%s` where the search terms go, such as
/// `https://github.com/search?q=%s`. Only http and https: a `javascript:` template would run
/// code in whatever page is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlTemplate(String);

impl UrlTemplate {
    /// The template as written, `%s` included.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The URL for `terms`, percent-encoded as a form value with spaces as `+`; `None` when the
    /// encoded terms make no URL, as when the `%s` stands in the host.
    #[must_use]
    pub fn fill(&self, terms: &str) -> Option<Url> {
        let encoded: String = form_urlencoded::byte_serialize(terms.as_bytes()).collect();
        Url::parse(&self.0.replace(PLACEHOLDER, &encoded)).ok()
    }
}

/// Why a text is not a URL template.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// The text has no `%s`.
    #[error("a template needs a %s for the terms")]
    NoPlaceholder,
    /// The text has more than one `%s`; holds how many.
    #[error("a template has one %s, not {0}")]
    SeveralPlaceholders(usize),
    /// The text, with terms in place, is not a URL.
    #[error("a template must be a URL: {0}")]
    NotAUrl(url::ParseError),
    /// The URL is not http or https; holds its scheme.
    #[error("a template must be an http or https URL, not {0}:")]
    NotWeb(String),
}

impl TryFrom<&str> for UrlTemplate {
    type Error = TemplateError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        match text.matches(PLACEHOLDER).count() {
            0 => return Err(TemplateError::NoPlaceholder),
            1 => {}
            n => return Err(TemplateError::SeveralPlaceholders(n)),
        }
        let sample =
            Url::parse(&text.replace(PLACEHOLDER, "terms")).map_err(TemplateError::NotAUrl)?;
        if !is_web(&sample) {
            return Err(TemplateError::NotWeb(sample.scheme().to_owned()));
        }
        Ok(Self(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{TemplateError, UrlTemplate};

    #[test]
    fn a_web_url_with_one_placeholder_is_a_template() {
        let text = "https://example.test/search?q=%s";

        let template = UrlTemplate::try_from(text);

        assert_eq!(template.as_ref().map(UrlTemplate::as_str), Ok(text));
    }

    #[test]
    fn an_http_template_is_accepted() {
        assert!(UrlTemplate::try_from("http://example.test/?q=%s").is_ok());
    }

    #[test]
    fn a_template_without_placeholder_is_rejected() {
        let result = UrlTemplate::try_from("https://example.test/");

        assert_eq!(result, Err(TemplateError::NoPlaceholder));
    }

    #[test]
    fn a_template_with_two_placeholders_is_rejected() {
        let result = UrlTemplate::try_from("https://example.test/%s?q=%s");

        assert_eq!(result, Err(TemplateError::SeveralPlaceholders(2)));
    }

    #[test]
    fn a_template_that_is_no_url_is_rejected() {
        let result = UrlTemplate::try_from("search %s");

        assert_eq!(
            result,
            Err(TemplateError::NotAUrl(
                url::ParseError::RelativeUrlWithoutBase
            ))
        );
    }

    #[test]
    fn a_template_outside_the_web_is_rejected() {
        let result = UrlTemplate::try_from("javascript:find('%s')");

        assert_eq!(result, Err(TemplateError::NotWeb("javascript".to_owned())));
    }

    #[test]
    fn fill_encodes_the_terms_as_a_form_value() {
        let template = UrlTemplate::try_from("https://example.test/?q=%s").unwrap();

        let url = template.fill("a&b=c ü");

        assert_eq!(
            url.as_ref().map(url::Url::as_str),
            Some("https://example.test/?q=a%26b%3Dc+%C3%BC")
        );
    }

    #[test]
    fn fill_gives_nothing_when_the_terms_break_the_url() {
        let template = UrlTemplate::try_from("https://%s.example.test/").unwrap();

        assert_eq!(template.fill("#"), None);
    }
}
