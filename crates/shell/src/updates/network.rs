use std::io;

use quickie_application::StopSignal;
use quickie_updater::{Response, Transport};
use url::Url;

pub(super) struct UpdateHttp(pub StopSignal);

impl Transport for UpdateHttp {
    #[cfg(feature = "main-webkit")]
    fn get(
        &self,
        url: &Url,
        accept: &str,
        body: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<Response, String> {
        use quickie_application::{Ended, Flow, HttpEvent, HttpMethod, HttpRequest};
        use quickie_engine_webkit::UrlSessionHttp;

        if self.0.is_stopped() {
            return Err("The update request was cancelled.".into());
        }
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: url.clone(),
            headers: vec![
                ("Accept".into(), accept.into()),
                ("User-Agent".into(), "Quickie".into()),
            ],
        };
        let mut status = None;
        let mut location = None;
        let mut sink_error = None;
        let ended = UrlSessionHttp::new().stream_with_location(
            &request,
            &self.0,
            &mut |event| {
                match event {
                    HttpEvent::Status(code) => status = Some(code),
                    HttpEvent::Body(bytes) if status == Some(200) => {
                        if let Err(error) = body(bytes) {
                            sink_error = Some(error);
                            return Flow::Stop;
                        }
                    }
                    HttpEvent::Body(_) => {}
                }
                Flow::Continue
            },
            &mut |redirect| location = Some(redirect),
        );
        if let Some(error) = sink_error {
            return Err(error.to_string());
        }
        match ended {
            Ok(Ended::Complete) => Ok(Response {
                status: status.ok_or("The update request did not receive an HTTP response.")?,
                location,
            }),
            Ok(Ended::Stopped) => Err("The update request was cancelled.".into()),
            Err(_) => Err("The update request could not reach GitHub.".into()),
        }
    }

    #[cfg(not(feature = "main-webkit"))]
    fn get(
        &self,
        _: &Url,
        _: &str,
        _: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<Response, String> {
        if self.0.is_stopped() {
            return Err("The update request was cancelled.".into());
        }
        Err("Updates require Quickie's WebKit network adapter.".into())
    }
}
