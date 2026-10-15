use crate::SlackError;
use dekopon_provider_sdk::provider::endpoint::Base;
use dekopon_provider_sdk::provider::{Header, HttpError, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    sections: Option<Vec<Section>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Section {
    id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupResult {
    canvas_id: String,
    sections: Vec<Section>,
}

pub fn lookup(
    base: &Base,
    canvas_id: &str,
    text: &str,
    mut send: impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<LookupResult, SlackError> {
    if text.trim().is_empty() {
        return Err(SlackError::NotFound);
    }
    let body = json!({"canvas_id": canvas_id, "criteria": {"contains_text": text}});
    let request = Request::new(
        "POST",
        base.join("/api/canvases.sections.lookup")
            .map_err(|_| SlackError::HttpFailed)?,
    )
    .map_err(|_| SlackError::HttpFailed)?
    .with_header(
        Header::text("content-type", "application/json").map_err(|_| SlackError::HttpFailed)?,
    )
    .with_body(serde_json::to_vec(&body).map_err(|_| SlackError::HttpFailed)?);
    let response = send(request).map_err(|_| SlackError::HttpFailed)?;
    if response.status != 200 {
        return Err(SlackError::HttpFailed);
    }
    let reply: Reply =
        serde_json::from_slice(&response.body).map_err(|_| SlackError::InvalidResponse)?;
    if !reply.ok {
        return Err(match reply.error.as_deref() {
            Some("canvas_not_found" | "canvas_deleted") => SlackError::NotFound,
            Some("access_denied" | "missing_scope" | "canvas_disabled_user_team") => {
                SlackError::AccessDenied
            }
            _ => SlackError::HttpFailed,
        });
    }
    let sections = reply.sections.ok_or(SlackError::InvalidResponse)?;
    if sections.is_empty() || sections.iter().any(|section| section.id.is_empty()) {
        return Err(if sections.is_empty() {
            SlackError::NotFound
        } else {
            SlackError::InvalidResponse
        });
    }
    Ok(LookupResult {
        canvas_id: canvas_id.to_owned(),
        sections,
    })
}
