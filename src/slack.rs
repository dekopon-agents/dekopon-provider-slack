use crate::{PostInput, SlackError};
use dekopon_provider_sdk::provider::endpoint::Base;
use dekopon_provider_sdk::provider::{Header, HttpError, HttpErrorCode, Request, Response};
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

fn pre_send_error(code: HttpErrorCode) -> HttpError {
    HttpError {
        code,
        message: String::new(),
    }
}

fn write_error(error: HttpError) -> SlackError {
    match error.code {
        HttpErrorCode::Timeout
        | HttpErrorCode::Protocol
        | HttpErrorCode::ResponseTooLarge
        | HttpErrorCode::Internal => SlackError::UnknownResult,
        HttpErrorCode::InvalidMethod
        | HttpErrorCode::InvalidUri
        | HttpErrorCode::InvalidHeader
        | HttpErrorCode::RequestTooLarge
        | HttpErrorCode::Denied
        | HttpErrorCode::HostCallLimit
        | HttpErrorCode::Dns
        | HttpErrorCode::Connect
        | HttpErrorCode::Tls => SlackError::HttpFailed,
    }
}

fn post(
    base: &Base,
    path: &str,
    body: Value,
    send: &mut impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<Response, HttpError> {
    let uri = base
        .join(path)
        .map_err(|_| pre_send_error(HttpErrorCode::InvalidUri))?;
    let request = Request::new("POST", uri)
        .map_err(|_| pre_send_error(HttpErrorCode::InvalidMethod))?
        .with_header(
            Header::text("content-type", "application/json")
                .map_err(|_| pre_send_error(HttpErrorCode::InvalidHeader))?,
        )
        .with_body(
            serde_json::to_vec(&body).map_err(|_| pre_send_error(HttpErrorCode::InvalidHeader))?,
        );
    send(request)
}

fn canvas_refusal(error: Option<&str>) -> SlackError {
    match error {
        Some("canvas_not_found" | "canvas_deleted") => SlackError::NotFound,
        Some("access_denied" | "missing_scope" | "canvas_disabled_user_team") => {
            SlackError::AccessDenied
        }
        _ => SlackError::HttpFailed,
    }
}

fn lookup_response(response: Response) -> Result<Vec<Section>, SlackError> {
    if response.status != 200 {
        return Err(SlackError::HttpFailed);
    }
    let reply: Reply =
        serde_json::from_slice(&response.body).map_err(|_| SlackError::InvalidResponse)?;
    if !reply.ok {
        return Err(canvas_refusal(reply.error.as_deref()));
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
    lookup_response(
        post(base, "/api/canvases.sections.lookup", body, send)
            .map_err(|_| SlackError::HttpFailed)?,
    )
}

pub fn lookup(
    base: &Base,
    canvas_id: &str,
    text: &str,
    mut send: impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<LookupResult, SlackError> {
    if text.trim().is_empty() {
        return Err(SlackError::InvalidInput);
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
    let sections = lookup_response(response).map_err(|_| SlackError::AcceptedUnverified)?;
    if sections.iter().any(|section| section.id == section_id) {
        Ok(())
    } else {
        Err(SlackError::AcceptedUnverified)
    }
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn single_block(markdown: &str) -> Result<&str, SlackError> {
    let mut lines = markdown
        .split(['\n', '\r'])
        .map(|line| line.trim_matches(is_blank))
        .filter(|line| !line.is_empty());
    let block = lines.next().ok_or(SlackError::InvalidInput)?;
    if lines.next().is_some() {
        return Err(SlackError::MultipleBlocks);
    }
    Ok(block)
}

fn ends_marker(rest: &str) -> bool {
    rest.is_empty() || rest.starts_with(is_blank)
}

fn strip_block_marker(mut line: &str) -> &str {
    while let Some(rest) = line.strip_prefix('>') {
        line = rest.trim_start_matches(is_blank);
    }
    if ['-', '*', '_'].iter().any(|&rule| {
        line.chars().filter(|&c| c == rule).count() >= 3
            && line.chars().all(|c| c == rule || is_blank(c))
    }) {
        return "";
    }
    let hashes = line.len() - line.trim_start_matches('#').len();
    if (1..=6).contains(&hashes) && ends_marker(&line[hashes..]) {
        return line[hashes..].trim_start_matches(is_blank);
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let item = if line.starts_with(['-', '*', '+']) && ends_marker(&line[1..]) {
        Some(&line[1..])
    } else if (1..=9).contains(&digits)
        && line[digits..].starts_with(['.', ')'])
        && ends_marker(&line[digits + 1..])
    {
        Some(&line[digits + 1..])
    } else {
        None
    };
    let Some(item) = item else {
        return line;
    };
    let item = item.trim_start_matches(is_blank);
    ["[ ]", "[x]", "[X]"]
        .iter()
        .find_map(|box_| item.strip_prefix(box_).filter(|rest| ends_marker(rest)))
        .unwrap_or(item)
        .trim_start_matches(is_blank)
}

fn verification_probe(block: &str) -> Result<String, SlackError> {
    let mut runs = vec![String::new()];
    let mut chars = strip_block_marker(block).chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ']' if chars.peek() == Some(&'(') => {
                chars.by_ref().find(|&c| c == ')');
                runs.push(String::new());
            }
            '<' => {
                chars.by_ref().find(|&c| c == '>');
                runs.push(String::new());
            }
            ':' => {
                let name = chars
                    .clone()
                    .take_while(|&c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
                    .count();
                if name > 0 && chars.clone().nth(name) == Some(':') {
                    chars.nth(name);
                }
                runs.push(String::new());
            }
            '*' | '_' | '`' | '~' | '[' | ']' | '>' | '!' | '\\' | '|' | '&' => {
                runs.push(String::new());
            }
            c => runs.last_mut().expect("one run").push(c),
        }
    }
    let (longest, _) =
        runs.iter()
            .map(|run| run.trim_matches(is_blank))
            .fold(("", 0), |(best, best_len), run| {
                let len = run.chars().count();
                if len > best_len {
                    (run, len)
                } else {
                    (best, best_len)
                }
            });
    let probe: String = longest.chars().take(64).collect();
    if !probe.chars().any(char::is_alphanumeric) {
        return Err(SlackError::InvalidInput);
    }
    Ok(probe)
}

pub fn replace(
    base: &Base,
    canvas_id: &str,
    section_match: &str,
    markdown: &str,
    mut send: impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<ReplaceResult, SlackError> {
    if section_match.trim().is_empty() {
        return Err(SlackError::InvalidInput);
    }
    let verify_text = verification_probe(single_block(markdown)?)?;
    let id = pre_edit(lookup_sections(base, canvas_id, section_match, &mut send)?)?;
    let body = json!({"canvas_id":canvas_id,"changes":[{"operation":"replace","section_id":id,"document_content":{"type":"markdown","markdown":markdown}}]});
    let response = post(base, "/api/canvases.edit", body, &mut send).map_err(write_error)?;
    edit_response(response)?;
    let verification = post(
        base,
        "/api/canvases.sections.lookup",
        json!({"canvas_id":canvas_id,"criteria":{"contains_text":verify_text}}),
        &mut send,
    )
    .map_err(|_| SlackError::AcceptedUnverified)?;
    verify_response(verification, &id)?;
    Ok(ReplaceResult {
        canvas_id: canvas_id.to_owned(),
        section_id: id,
    })
}

#[derive(Deserialize)]
struct ContentReply {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    content: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResult {
    canvas_id: String,
    markdown: String,
}

fn read_error(error: HttpError) -> SlackError {
    match error.code {
        HttpErrorCode::ResponseTooLarge => SlackError::TooLarge,
        _ => SlackError::HttpFailed,
    }
}

pub fn read(
    base: &Base,
    canvas_id: &str,
    mut send: impl FnMut(Request) -> Result<Response, HttpError>,
) -> Result<ReadResult, SlackError> {
    let body = json!({"canvas_id": canvas_id, "content_type": "markdown"});
    let response = post(base, "/api/canvases.getContent", body, &mut send).map_err(read_error)?;
    if response.status != 200 {
        return Err(SlackError::HttpFailed);
    }
    let reply: ContentReply =
        serde_json::from_slice(&response.body).map_err(|_| SlackError::InvalidResponse)?;
    if !reply.ok {
        return Err(canvas_refusal(reply.error.as_deref()));
    }
    match reply.content {
        Some(Value::String(markdown)) => Ok(ReadResult {
            canvas_id: canvas_id.to_owned(),
            markdown,
        }),
        _ => Err(SlackError::InvalidResponse),
    }
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
    let response = post(base, "/api/chat.postMessage", body, &mut send).map_err(write_error)?;
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
    fn native_post_connect_refusal_is_http_failed_and_timeout_is_unknown_without_retry() {
        let base = Base::from_static("https://slack.com");
        let input = PostInput {
            channel: "C123".into(),
            text: "hello".into(),
            thread_ts: Some("123.000001".into()),
        };
        for (code, unknown) in [
            (HttpErrorCode::Connect, false),
            (HttpErrorCode::Timeout, true),
        ] {
            let mut requests = Vec::new();
            let result = post_message(&base, &input, |request| {
                requests.push(request);
                Err(HttpError {
                    code,
                    message: "simulated failure".into(),
                })
            });
            assert!(if unknown {
                matches!(result, Err(SlackError::UnknownResult))
            } else {
                matches!(result, Err(SlackError::HttpFailed))
            });
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            assert_eq!(request.method, "POST");
            assert_eq!(
                request.uri.as_str(),
                "https://slack.com/api/chat.postMessage"
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&request.body).expect("body"),
                json!({"channel":"C123","text":"hello","thread_ts":"123.000001"})
            );
            assert!(
                !request
                    .headers
                    .iter()
                    .any(|h| h.name.eq_ignore_ascii_case("authorization"))
            );
        }
    }

    #[test]
    fn native_verification_connect_refusal_after_accepted_edit_is_accepted_unverified_without_retry()
     {
        let base = Base::from_static("https://slack.com");
        let mut requests = Vec::new();
        let result = replace(&base, "canvas", "old", "new", |request| {
            requests.push(request);
            match requests.len() {
                1 => Ok(response(json!({"ok":true,"sections":[{"id":"one"}]}))),
                2 => Ok(response(json!({"ok":true}))),
                _ => Err(HttpError {
                    code: HttpErrorCode::Connect,
                    message: "connection refused".into(),
                }),
            }
        });
        assert!(matches!(result, Err(SlackError::AcceptedUnverified)));
        assert_eq!(requests.len(), 3);
        for (request, (path, body)) in requests.iter().zip([
            ("/api/canvases.sections.lookup", json!({"canvas_id":"canvas","criteria":{"contains_text":"old"}})),
            ("/api/canvases.edit", json!({"canvas_id":"canvas","changes":[{"operation":"replace","section_id":"one","document_content":{"type":"markdown","markdown":"new"}}]})),
            ("/api/canvases.sections.lookup", json!({"canvas_id":"canvas","criteria":{"contains_text":"new"}})),
        ]) {
            assert_eq!(request.uri, format!("https://slack.com{path}"));
            assert_eq!(request.method, "POST");
            assert_eq!(serde_json::from_slice::<Value>(&request.body).expect("body"), body);
            assert!(!request.headers.iter().any(|h| h.name.eq_ignore_ascii_case("authorization")));
        }
    }

    #[test]
    fn native_sequence_failures_do_not_select_or_retry_a_section() {
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
            Err(SlackError::AcceptedUnverified)
        ));
        assert!(matches!(
            verify_response(
                response(json!({"ok":false,"sections":[{"id":"original"}]})),
                "original"
            ),
            Err(SlackError::AcceptedUnverified)
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
    fn native_verification_passes_when_the_edited_id_is_one_of_two_matches() {
        for sections in [
            json!([{"id":"other"},{"id":"edited"}]),
            json!([{"id":"edited"},{"id":"other"}]),
        ] {
            assert!(
                verify_response(response(json!({"ok":true,"sections":sections})), "edited").is_ok()
            );
        }
    }

    #[test]
    fn native_markdown_of_more_than_one_block_refuses_multiple_blocks_and_one_line_is_one_block() {
        for markdown in [
            "## Log\n\nLog line one: synthetic.\n\nmac-verify after 2026-11-07",
            "## Status\nStatus: green",
            "first paragraph\n\nsecond paragraph",
            "- one\n- two",
            "| a | b |\n| --- | --- |\n| 1 | 2 |",
            "first\rsecond",
        ] {
            let error = single_block(markdown).expect_err("more than one block");
            assert!(matches!(error, SlackError::MultipleBlocks));
            assert_eq!(crate::Failure::code(&error).as_str(), "multiple-blocks");
            assert_eq!(crate::Failure::code(&error).status().get(), 2);
        }
        for (markdown, block) in [
            ("mac-verify after 2026-11-07", "mac-verify after 2026-11-07"),
            ("\n  ## Log  \n\n", "## Log"),
            ("- one", "- one"),
        ] {
            assert_eq!(single_block(markdown).expect("one block"), block);
        }
        assert!(matches!(
            single_block(" \n\t\r"),
            Err(SlackError::InvalidInput)
        ));
    }

    #[test]
    fn native_only_spaces_and_tabs_are_blank_so_a_no_break_space_line_is_its_own_block() {
        assert!(matches!(
            single_block("\u{a0}\nmac-verify after 2026-11-07"),
            Err(SlackError::MultipleBlocks)
        ));
        assert_eq!(single_block(" \t\u{a0}\t ").expect("one block"), "\u{a0}");
        assert_eq!(
            single_block("\t \nmac-verify after\t\n \t").expect("one block"),
            "mac-verify after"
        );
    }

    #[test]
    fn native_verification_probe_is_the_longest_plain_text_run_after_the_block_marker() {
        for (block, probe) in [
            ("mac-verify after 2026-11-07", "mac-verify after 2026-11-07"),
            ("## Log", "Log"),
            ("### Log v2", "Log v2"),
            ("#hashtag stays", "#hashtag stays"),
            ("- [x] Ship the fix", "Ship the fix"),
            ("12. Twelfth item", "Twelfth item"),
            ("> > Quoted twice", "Quoted twice"),
            ("Ship **the provider fix** today", "the provider fix"),
            (
                "See [the release plan](https://example.invalid/a-much-longer-url-path) now",
                "the release plan",
            ),
            (
                "Mail <https://example.invalid/very-long-autolink-path> to Ann",
                "to Ann",
            ),
            ("Status: green :white_check_mark:", "Status"),
        ] {
            assert_eq!(verification_probe(block).expect("probe"), probe, "{block}");
        }
        assert_eq!(
            verification_probe(&"x".repeat(80))
                .expect("probe")
                .chars()
                .count(),
            64
        );
        for block in [
            "---", "* * *", ":tada:", "**", "## ", "##", "######", "-", "*", "+", "1.", "2)", ">",
            "- [ ]", "- [x]", "1. [X]", "- #", "- ## ", "1. ##", "- [ ] #", "## ##", "# #",
            "- - #", "> - #", "...",
        ] {
            assert!(
                matches!(verification_probe(block), Err(SlackError::InvalidInput)),
                "{block}"
            );
        }
    }

    #[test]
    fn native_markup_only_block_whose_trailing_space_is_trimmed_refuses_invalid_input_with_zero_requests()
     {
        let base = Base::from_static("https://slack.com");
        for markdown in [
            "## ", "######\t", "- ", "1. ", "- [ ] ", "> ", "- #", "- ## ", "1. ##", "- [ ] #",
            "## ##", "# #", "- - #", "> - #", "...",
        ] {
            let mut requests = Vec::new();
            let result = replace(&base, "canvas", "Log", markdown, |request| {
                requests.push(request);
                Ok(response(json!({"ok":true,"sections":[{"id":"one"}]})))
            });
            assert!(
                matches!(result, Err(SlackError::InvalidInput)),
                "{markdown:?}"
            );
            assert!(requests.is_empty(), "{markdown:?}");
        }
    }

    #[test]
    fn native_probe_is_checked_after_truncation_so_periods_before_a_late_letter_refuse_with_zero_requests()
     {
        let base = Base::from_static("https://slack.com");
        for markdown in [
            format!("{}a", ".".repeat(64)),
            format!("{} a", ".".repeat(63)),
            format!("## {}1", ".".repeat(64)),
            format!("{}\u{e9}", "\u{2026}".repeat(64)),
        ] {
            assert!(matches!(
                verification_probe(single_block(&markdown).expect("one block")),
                Err(SlackError::InvalidInput)
            ));
            let mut requests = Vec::new();
            let result = replace(&base, "canvas", "Log", &markdown, |request| {
                requests.push(request);
                Ok(response(json!({"ok":true,"sections":[{"id":"one"}]})))
            });
            assert!(
                matches!(result, Err(SlackError::InvalidInput)),
                "{markdown:?}"
            );
            assert!(requests.is_empty(), "{markdown:?}");
        }
    }

    #[test]
    fn native_heading_replacement_verifies_with_the_heading_text_not_its_markup() {
        let base = Base::from_static("https://slack.com");
        let mut requests = Vec::new();
        let result = replace(&base, "canvas", "Log", "## Log v2", |request| {
            requests.push(request);
            Ok(response(match requests.len() {
                2 => json!({"ok":true}),
                _ => json!({"ok":true,"sections":[{"id":"heading"}]}),
            }))
        })
        .expect("replace");
        assert_eq!(result.section_id, "heading");
        assert_eq!(requests.len(), 3);
        for (request, (path, body)) in requests.iter().zip([
            ("/api/canvases.sections.lookup", json!({"canvas_id":"canvas","criteria":{"contains_text":"Log"}})),
            ("/api/canvases.edit", json!({"canvas_id":"canvas","changes":[{"operation":"replace","section_id":"heading","document_content":{"type":"markdown","markdown":"## Log v2"}}]})),
            ("/api/canvases.sections.lookup", json!({"canvas_id":"canvas","criteria":{"contains_text":"Log v2"}})),
        ]) {
            assert_request(request, path, &body);
        }
    }

    #[test]
    fn native_replace_failures_are_typed_by_stage_and_never_send_a_fourth_request() {
        let base = Base::from_static("https://slack.com");
        let lookup = response(json!({"ok":true,"sections":[{"id":"one"}]}));
        for (edit, verification, count, variant, code) in [
            (
                response(json!({"ok":false,"error":"canvas_editing_locked"})),
                None,
                2,
                SlackError::Conflict,
                "conflict",
            ),
            (
                response(json!({"ok":false,"error":"fatal_error"})),
                None,
                2,
                SlackError::UnknownResult,
                "unknown-result",
            ),
            (
                status(500, r#"{"ok":true}"#),
                None,
                2,
                SlackError::UnknownResult,
                "unknown-result",
            ),
            (
                status(200, "not json"),
                None,
                2,
                SlackError::UnknownResult,
                "unknown-result",
            ),
            (
                response(json!({"ok":true})),
                Some(response(json!({"ok":true,"sections":[{"id":"changed"}]}))),
                3,
                SlackError::AcceptedUnverified,
                "accepted-unverified",
            ),
            (
                response(json!({"ok":true})),
                Some(response(json!({"ok":true,"sections":[]}))),
                3,
                SlackError::AcceptedUnverified,
                "accepted-unverified",
            ),
            (
                response(json!({"ok":true})),
                Some(response(json!({"ok":false,"sections":[{"id":"one"}]}))),
                3,
                SlackError::AcceptedUnverified,
                "accepted-unverified",
            ),
            (
                response(json!({"ok":true})),
                Some(status(500, r#"{"ok":true,"sections":[{"id":"one"}]}"#)),
                3,
                SlackError::AcceptedUnverified,
                "accepted-unverified",
            ),
        ] {
            let mut replies = vec![lookup.clone(), edit];
            replies.extend(verification);
            let mut requests = Vec::new();
            let error = replace(&base, "canvas", "old", "new", |request| {
                requests.push(request);
                Ok(replies
                    .get(requests.len() - 1)
                    .cloned()
                    .unwrap_or_else(|| lookup.clone()))
            })
            .err()
            .expect("refusal");
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&variant)
            );
            assert_eq!(crate::Failure::code(&error).as_str(), code);
            assert_eq!(requests.len(), count);
            for (request, (path, body)) in requests.iter().zip([
                ("/api/canvases.sections.lookup", json!({"canvas_id":"canvas","criteria":{"contains_text":"old"}})),
                ("/api/canvases.edit", json!({"canvas_id":"canvas","changes":[{"operation":"replace","section_id":"one","document_content":{"type":"markdown","markdown":"new"}}]})),
                ("/api/canvases.sections.lookup", json!({"canvas_id":"canvas","criteria":{"contains_text":"new"}})),
            ]) {
                assert_request(request, path, &body);
            }
        }
    }

    #[test]
    fn native_edit_transport_failure_is_unknown_result_without_verification_or_retry() {
        let base = Base::from_static("https://slack.com");
        let mut requests = Vec::new();
        let error = replace(&base, "canvas", "old", "new", |request| {
            requests.push(request);
            if requests.len() == 1 {
                Ok(response(json!({"ok":true,"sections":[{"id":"one"}]})))
            } else {
                Err(HttpError {
                    code: HttpErrorCode::Timeout,
                    message: "simulated timeout".into(),
                })
            }
        })
        .err()
        .expect("refusal");
        assert!(matches!(error, SlackError::UnknownResult));
        assert_eq!(crate::Failure::code(&error).as_str(), "unknown-result");
        assert_eq!(requests.len(), 2);
        assert_request(
            &requests[0],
            "/api/canvases.sections.lookup",
            &json!({"canvas_id":"canvas","criteria":{"contains_text":"old"}}),
        );
        assert_request(
            &requests[1],
            "/api/canvases.edit",
            &json!({"canvas_id":"canvas","changes":[{"operation":"replace","section_id":"one","document_content":{"type":"markdown","markdown":"new"}}]}),
        );
    }

    fn assert_request(request: &Request, path: &str, body: &Value) {
        assert_eq!(request.uri, format!("https://slack.com{path}"));
        assert_eq!(request.method, "POST");
        assert_eq!(
            &serde_json::from_slice::<Value>(&request.body).expect("body"),
            body
        );
        assert!(
            !request
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("authorization"))
        );
    }

    fn status(code: u16, body: &str) -> Response {
        Response {
            status: code,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn native_read_sends_one_markdown_get_content_and_maps_each_refusal_to_its_typed_code() {
        use std::mem::discriminant;
        let base = Base::from_static("https://slack.com");
        let cases: [(Result<Response, HttpErrorCode>, SlackError, &str); 13] = [
            (
                Ok(status(500, r#"{"ok":true,"content":"x"}"#)),
                SlackError::HttpFailed,
                "http-failed",
            ),
            (
                Ok(response(json!({"ok":false,"error":"ratelimited"}))),
                SlackError::HttpFailed,
                "http-failed",
            ),
            (
                Ok(response(json!({"ok":false}))),
                SlackError::HttpFailed,
                "http-failed",
            ),
            (
                Ok(response(json!({"ok":false,"error":"canvas_not_found"}))),
                SlackError::NotFound,
                "not-found",
            ),
            (
                Ok(response(json!({"ok":false,"error":"canvas_deleted"}))),
                SlackError::NotFound,
                "not-found",
            ),
            (
                Ok(response(json!({"ok":false,"error":"access_denied"}))),
                SlackError::AccessDenied,
                "access-denied",
            ),
            (
                Ok(response(json!({"ok":false,"error":"missing_scope"}))),
                SlackError::AccessDenied,
                "access-denied",
            ),
            (
                Ok(response(
                    json!({"ok":false,"error":"canvas_disabled_user_team"}),
                )),
                SlackError::AccessDenied,
                "access-denied",
            ),
            (
                Ok(status(200, "not json")),
                SlackError::InvalidResponse,
                "invalid-response",
            ),
            (
                Ok(response(json!({"ok":true}))),
                SlackError::InvalidResponse,
                "invalid-response",
            ),
            (
                Ok(response(json!({"ok":true,"content":7}))),
                SlackError::InvalidResponse,
                "invalid-response",
            ),
            (
                Err(HttpErrorCode::ResponseTooLarge),
                SlackError::TooLarge,
                "too-large",
            ),
            (
                Err(HttpErrorCode::Timeout),
                SlackError::HttpFailed,
                "http-failed",
            ),
        ];
        for (reply, expected, code) in cases {
            let mut requests = Vec::new();
            let error = read(&base, "F0SYNTH", |request| {
                requests.push(request);
                reply.clone().map_err(|code| HttpError {
                    code,
                    message: "simulated failure".into(),
                })
            })
            .err()
            .expect("refusal");
            assert_eq!(discriminant(&error), discriminant(&expected));
            assert_eq!(crate::Failure::code(&error).as_str(), code);
            assert_eq!(requests.len(), 1);
            assert_request(
                &requests[0],
                "/api/canvases.getContent",
                &json!({"canvas_id":"F0SYNTH","content_type":"markdown"}),
            );
        }
    }

    #[test]
    fn native_read_lookup_replace_read_cycle_over_a_repeated_heading_edits_only_the_looked_up_block_of_a_synthetic_canvas_not_live_slack()
     {
        let base = Base::from_static("https://slack.com");
        let before_block = "mac-verify before";
        let after_block = "mac-verify after 2026-11-07";
        let mut canvas = [
            ("temp:C:aaa".to_owned(), "# Plan".to_owned()),
            ("temp:C:bbb".to_owned(), "## Log".to_owned()),
            (
                "temp:C:ccc".to_owned(),
                "Log line one: synthetic canvas created.".to_owned(),
            ),
            ("temp:C:ddd".to_owned(), before_block.to_owned()),
            ("temp:C:eee".to_owned(), "## Log".to_owned()),
            (
                "temp:C:fff".to_owned(),
                "Archive: mac-verify after 2026-11-07 was planned.".to_owned(),
            ),
        ];
        let rendered = |canvas: &[(String, String)]| {
            canvas
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let before_canvas = rendered(&canvas);
        let mut requests: Vec<Request> = Vec::new();
        let mut send = |request: Request| -> Result<Response, HttpError> {
            let body: Value = serde_json::from_slice(&request.body).expect("body");
            let path = request
                .uri
                .trim_start_matches("https://slack.com")
                .to_owned();
            requests.push(request);
            Ok(response(match path.as_str() {
                "/api/canvases.getContent" => json!({"ok":true,"content":rendered(&canvas)}),
                "/api/canvases.sections.lookup" => {
                    let needle = body["criteria"]["contains_text"]
                        .as_str()
                        .expect("criteria");
                    let ids: Vec<Value> = canvas
                        .iter()
                        .filter(|(_, text)| text.contains(needle))
                        .map(|(id, _)| json!({"id": id}))
                        .collect();
                    json!({"ok":true,"sections":ids})
                }
                "/api/canvases.edit" => {
                    let change = &body["changes"][0];
                    let id = change["section_id"].as_str().expect("section");
                    let markdown = change["document_content"]["markdown"]
                        .as_str()
                        .expect("markdown");
                    let section = canvas
                        .iter_mut()
                        .find(|(sid, _)| sid == id)
                        .expect("known section");
                    section.1 = markdown.to_owned();
                    json!({"ok":true})
                }
                other => panic!("unexpected path {other}"),
            }))
        };
        let before = read(&base, "F0SYNTH", &mut send).expect("first read");
        let looked_up = lookup(&base, "F0SYNTH", before_block, &mut send).expect("lookup");
        let refused = replace(
            &base,
            "F0SYNTH",
            before_block,
            &format!("## Log\n\nLog line one: synthetic canvas created.\n\n{after_block}"),
            &mut send,
        );
        let replaced =
            replace(&base, "F0SYNTH", before_block, after_block, &mut send).expect("replace");
        let after = read(&base, "F0SYNTH", &mut send).expect("second read");

        assert!(matches!(refused, Err(SlackError::MultipleBlocks)));
        assert_eq!(before.markdown, before_canvas);
        assert_eq!(
            after.markdown,
            before_canvas.replacen(before_block, after_block, 1)
        );
        assert_eq!(after.markdown.matches("## Log").count(), 2);
        assert_eq!(after.markdown.matches(after_block).count(), 2);
        let sections: Vec<&str> = looked_up.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(sections, ["temp:C:ddd"]);
        assert_eq!(replaced.section_id, "temp:C:ddd");
        assert_eq!(requests.len(), 6);
        let read_body = json!({"canvas_id":"F0SYNTH","content_type":"markdown"});
        let lookup_body = json!({"canvas_id":"F0SYNTH","criteria":{"contains_text":before_block}});
        for (request, (path, body)) in requests.iter().zip([
            ("/api/canvases.getContent", read_body.clone()),
            ("/api/canvases.sections.lookup", lookup_body.clone()),
            ("/api/canvases.sections.lookup", lookup_body),
            ("/api/canvases.edit", json!({"canvas_id":"F0SYNTH","changes":[{"operation":"replace","section_id":"temp:C:ddd","document_content":{"type":"markdown","markdown":after_block}}]})),
            ("/api/canvases.sections.lookup", json!({"canvas_id":"F0SYNTH","criteria":{"contains_text":after_block}})),
            ("/api/canvases.getContent", read_body),
        ]) {
            assert_request(request, path, &body);
        }
    }
}
