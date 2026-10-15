use crate::{PostInput, SlackError};
use dekopon_provider_sdk::provider::endpoint::Base;
use dekopon_provider_sdk::provider::{Header, HttpError, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

fn post(
    base: &Base,
    path: &str,
    body: Value,
    send: &mut impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<Response, SlackError> {
    let request = Request::new("POST", base.join(path).map_err(|_| SlackError::HttpFailed)?)
        .map_err(|_| SlackError::HttpFailed)?
        .with_header(
            Header::text("content-type", "application/json").map_err(|_| SlackError::HttpFailed)?,
        )
        .with_body(serde_json::to_vec(&body).map_err(|_| SlackError::HttpFailed)?);
    send(request).map_err(|_| SlackError::HttpFailed)
}

fn lookup_response(response: Response) -> Result<Vec<Section>, SlackError> {
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
    if sections.iter().any(|section| section.id.is_empty()) {
        return Err(SlackError::InvalidResponse);
    }
    Ok(sections)
}

fn lookup_sections(
    base: &Base,
    canvas_id: &str,
    text: &str,
    send: &mut impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<Vec<Section>, SlackError> {
    let body = json!({"canvas_id": canvas_id, "criteria": {"contains_text": text}});
    lookup_response(post(base, "/api/canvases.sections.lookup", body, send)?)
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
    let sections = lookup_sections(base, canvas_id, text, &mut send)?;
    if sections.is_empty() {
        return Err(SlackError::NotFound);
    }
    Ok(LookupResult {
        canvas_id: canvas_id.to_owned(),
        sections,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceResult {
    canvas_id: String,
    section_id: String,
}

fn pre_edit(sections: Vec<Section>) -> Result<String, SlackError> {
    match sections.as_slice() {
        [] => Err(SlackError::NotFound),
        [section] => Ok(section.id.clone()),
        _ => Err(SlackError::Ambiguous),
    }
}

fn edit_response(response: Response) -> Result<(), SlackError> {
    if response.status != 200 {
        return Err(SlackError::UnknownResult);
    }
    let reply: Reply =
        serde_json::from_slice(&response.body).map_err(|_| SlackError::UnknownResult)?;
    if reply.ok {
        return Ok(());
    }
    Err(match reply.error.as_deref() {
        Some(
            "canvas_editing_locked" | "canvas_locked" | "section_not_found" | "canvas_not_found",
        ) => SlackError::Conflict,
        _ => SlackError::UnknownResult,
    })
}

fn verify_response(response: Response, section_id: &str) -> Result<(), SlackError> {
    let sections = lookup_response(response).map_err(|_| SlackError::UnknownResult)?;
    if sections.len() == 1 && sections[0].id == section_id {
        Ok(())
    } else {
        Err(SlackError::UnknownResult)
    }
}

pub fn replace(
    base: &Base,
    canvas_id: &str,
    section_match: &str,
    markdown: &str,
    mut send: impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<ReplaceResult, SlackError> {
    if section_match.trim().is_empty() {
        return Err(SlackError::NotFound);
    }
    let line = markdown
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or(SlackError::InvalidInput)?;
    let verify_text: String = line.trim().chars().take(64).collect();
    let id = pre_edit(lookup_sections(base, canvas_id, section_match, &mut send)?)?;
    let body = json!({"canvas_id":canvas_id,"changes":[{"operation":"replace","section_id":id,"document_content":{"type":"markdown","markdown":markdown}}]});
    let response =
        post(base, "/api/canvases.edit", body, &mut send).map_err(|_| SlackError::UnknownResult)?;
    edit_response(response)?;
    let verification = post(
        base,
        "/api/canvases.sections.lookup",
        json!({"canvas_id":canvas_id,"criteria":{"contains_text":verify_text}}),
        &mut send,
    )
    .map_err(|_| SlackError::UnknownResult)?;
    verify_response(verification, &id)?;
    Ok(ReplaceResult {
        canvas_id: canvas_id.to_owned(),
        section_id: id,
    })
}

#[derive(Serialize)]
pub struct PostResult {
    channel: String,
    ts: String,
}

fn valid_ts(ts: &str) -> bool {
    ts.split_once('.').is_some_and(|(seconds, fraction)| {
        !seconds.is_empty()
            && seconds.bytes().all(|b| b.is_ascii_digit())
            && fraction.len() == 6
            && fraction.bytes().all(|b| b.is_ascii_digit())
    })
}

pub fn post_message(
    base: &Base,
    input: &PostInput,
    mut send: impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<PostResult, SlackError> {
    if input.channel.trim().is_empty() || input.text.trim().is_empty() {
        return Err(SlackError::InvalidInput);
    }
    if input.thread_ts.as_deref().is_some_and(|ts| !valid_ts(ts)) {
        return Err(SlackError::InvalidTimestamp);
    }
    let mut body = json!({"channel":input.channel,"text":input.text});
    if let Some(ts) = &input.thread_ts {
        body["thread_ts"] = json!(ts);
    }
    let response = post(base, "/api/chat.postMessage", body, &mut send)
        .map_err(|_| SlackError::UnknownResult)?;
    if response.status != 200 {
        return Err(SlackError::UnknownResult);
    }
    let reply: Value =
        serde_json::from_slice(&response.body).map_err(|_| SlackError::UnknownResult)?;
    if reply.get("ok") == Some(&json!(false)) {
        return Err(SlackError::PostRefused);
    }
    if reply.get("ok") != Some(&json!(true)) {
        return Err(SlackError::UnknownResult);
    }
    let channel = reply
        .get("channel")
        .and_then(Value::as_str)
        .filter(|channel| !channel.is_empty())
        .ok_or(SlackError::UnknownResult)?;
    let ts = reply
        .get("ts")
        .and_then(Value::as_str)
        .filter(|ts| valid_ts(ts))
        .ok_or(SlackError::UnknownResult)?;
    Ok(PostResult {
        channel: channel.into(),
        ts: ts.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(value: Value) -> Response {
        Response {
            status: 200,
            headers: vec![],
            body: value.to_string().into_bytes(),
        }
    }

    #[test]
    fn sequence_failures_do_not_select_or_retry_a_section() {
        assert!(matches!(
            pre_edit(vec![Section { id: "a".into() }, Section { id: "b".into() }]),
            Err(SlackError::Ambiguous)
        ));
        for error in ["canvas_editing_locked", "section_not_found"] {
            assert!(matches!(
                edit_response(response(json!({"ok":false,"error":error}))),
                Err(SlackError::Conflict)
            ));
        }
        assert!(matches!(
            edit_response(response(json!({"ok":false,"error":"fatal_error"}))),
            Err(SlackError::UnknownResult)
        ));
        assert!(matches!(
            verify_response(
                response(json!({"ok":true,"sections":[{"id":"changed"}]})),
                "original"
            ),
            Err(SlackError::UnknownResult)
        ));
        assert!(matches!(
            verify_response(
                response(json!({"ok":false,"sections":[{"id":"original"}]})),
                "original"
            ),
            Err(SlackError::UnknownResult)
        ));
        assert!(
            verify_response(
                response(json!({"ok":true,"sections":[{"id":"original"}]})),
                "original"
            )
            .is_ok()
        );
    }

    #[test]
    fn replace_stops_after_sequence_failure_without_retry() {
        let base = Base::from_static("https://slack.com");
        for (edit, verification, count, conflict) in [
            (
                json!({"ok":false,"error":"canvas_editing_locked"}),
                json!({"ok":true,"sections":[{"id":"one"}]}),
                2,
                true,
            ),
            (
                json!({"ok":false,"error":"fatal_error"}),
                json!({"ok":true,"sections":[{"id":"one"}]}),
                2,
                false,
            ),
            (
                json!({"ok":true}),
                json!({"ok":true,"sections":[{"id":"changed"}]}),
                3,
                false,
            ),
            (
                json!({"ok":true}),
                json!({"ok":false,"sections":[{"id":"one"}]}),
                3,
                false,
            ),
        ] {
            let replies = [
                json!({"ok":true,"sections":[{"id":"one"}]}),
                edit,
                verification,
            ];
            let mut calls = 0;
            let result = replace(&base, "canvas", "old", "new", |_| {
                let result = response(replies[calls].clone());
                calls += 1;
                Ok(result)
            });
            if conflict {
                assert!(matches!(result, Err(SlackError::Conflict)));
            } else {
                assert!(matches!(result, Err(SlackError::UnknownResult)));
            }
            assert_eq!(calls, count);
        }
    }
}
