use dekopon_provider_sdk::provider::{Http, ImportSet, Needs, Provider, Response};
use dekopon_provider_sdk::{CommandRunOutcome, provider};
use dekopon_provider_sdk_testkit::{Harness, HttpScript, Native, conformance};
use dekopon_slack_provider::Slack;
use serde_json::{Value, json};

const CANVAS: &str = "F0C907QUHU7";

fn component() -> std::path::PathBuf {
    std::env::var_os("DEKOPON_PROVIDER_COMPONENT")
        .expect("DEKOPON_PROVIDER_COMPONENT must name the freshly built component")
        .into()
}

fn reply(value: Value) -> Response {
    Response {
        status: 200,
        headers: vec![],
        body: value.to_string().into_bytes(),
    }
}

#[test]
fn manifest_declares_only_section_lookup() -> Result<(), Box<dyn std::error::Error>> {
    conformance::<Slack>(component())?;
    let manifest = provider::manifest::<Slack>()?;
    assert_eq!(manifest.capabilities.len(), 1);
    assert_eq!(manifest.capabilities[0].id.as_str(), "slack.canvas.lookup");
    assert_eq!(
        manifest.capabilities[0].input_schema["additionalProperties"],
        false
    );
    assert!(<Http as Needs>::IMPORTS.contains(ImportSet::HTTP));
    assert_eq!(<Slack as Provider>::COMMAND_WORDS, &["slack"]);
    Ok(())
}

#[test]
fn lookup_proposes_typed_input_and_content_read_refuses() {
    let args = ["canvas", "lookup", "Status"].map(str::to_owned);
    let CommandRunOutcome::Proposed {
        capability, input, ..
    } = provider::command::<Slack>(&args, false)
    else {
        panic!("lookup must propose")
    };
    assert_eq!(capability.as_str(), "slack.canvas.lookup");
    assert_eq!(input, json!({"containsText":"Status"}));
    assert!(matches!(
        provider::command::<Slack>(&["canvas".into(), "read".into()], false),
        CommandRunOutcome::Failed { .. }
    ));
}

#[test]
fn lookup_returns_every_section_id_without_content() -> Result<(), Box<dyn std::error::Error>> {
    let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
        "slack.com",
        "POST",
        reply(json!({"ok":true,"sections":[{"id":"temp:C:one"},{"id":"temp:C:two"}]})),
    ));
    let origin = harness.origin().expect("script origin").to_owned();
    let output = harness
        .settings(json!({"canvasId":CANVAS,"baseUrl":origin}))
        .call("slack.canvas.lookup", json!({"containsText":"Status"}))?;
    assert_eq!(output.status, 0, "{}", output.stderr);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?,
        json!({"canvasId":CANVAS,"sections":[{"id":"temp:C:one"},{"id":"temp:C:two"}]})
    );
    assert_eq!(output.http_calls.len(), 1);
    let request = output.http_request.as_ref().expect("one recorded request");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.uri,
        format!("{origin}/api/canvases.sections.lookup")
    );
    assert!(
        request
            .headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("content-type")
                && header.value == b"application/json")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({"canvas_id":CANVAS,"criteria":{"contains_text":"Status"}})
    );
    assert!(
        !request
            .headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("authorization"))
    );
    Ok(())
}

#[test]
fn native_default_origin_is_slack_com() -> Result<(), Box<dyn std::error::Error>> {
    let native = Native::<Slack>::new()
        .settings(json!({"canvasId":CANVAS}))
        .http(HttpScript::new(
            "slack.com",
            "POST",
            reply(json!({"ok":true,"sections":[{"id":"temp:C:one"}]})),
        ));
    let output = native.call(
        "slack.canvas.lookup",
        &json!({"containsText":"Status"}).to_string(),
    );
    assert_eq!(output.status, 0, "{}", output.stderr);
    let requests = native.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].uri,
        "https://slack.com/api/canvases.sections.lookup"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body)?,
        json!({"canvas_id":CANVAS,"criteria":{"contains_text":"Status"}})
    );
    assert!(
        !requests[0]
            .headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("authorization"))
    );
    Ok(())
}

#[test]
fn malformed_canvas_settings_refuse_before_http() -> Result<(), Box<dyn std::error::Error>> {
    for settings in [
        None,
        Some(json!({"canvasId":null})),
        Some(json!({"canvasId":""})),
        Some(json!({"canvasId":12})),
        Some(json!({"canvasId":CANVAS,"baseUrl":null})),
        Some(json!({"canvasId":CANVAS,"baseUrl":"http://slack.com"})),
        Some(json!({"canvasId":CANVAS,"baseUrl":"https://slack.com/other"})),
        Some(json!({"canvasId":CANVAS,"baseUrl":"https://user@slack.com"})),
        Some(json!({"canvasId":CANVAS,"baseUrl":"https://slack.com:0"})),
        Some(json!({"canvasId":CANVAS,"baseUrl":"https://-slack.com"})),
        Some(json!({"canvasId":CANVAS,"baseUrl":"https://127.0.0.1"})),
    ] {
        let mut harness = Harness::<Slack>::get(component());
        if let Some(settings) = settings {
            harness = harness.settings(settings);
        }
        let output = harness.call("slack.canvas.lookup", json!({"containsText":"Status"}))?;
        assert_eq!(output.status, 1);
        assert!(output.stderr.contains("settings"), "{}", output.stderr);
        assert!(output.http_calls.is_empty());
    }
    Ok(())
}

#[test]
fn zero_matches_and_inaccessible_canvas_refuse() -> Result<(), Box<dyn std::error::Error>> {
    for (reply_body, error) in [
        (json!({"ok":true,"sections":[]}), "no matching"),
        (
            json!({"ok":false,"error":"access_denied"}),
            "not accessible",
        ),
        (
            json!({"ok":false,"error":"canvas_not_found"}),
            "not available",
        ),
        (
            json!({"ok":false,"sections":[{"id":"temp:C:one"}]}),
            "request failed",
        ),
    ] {
        let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
            "slack.com",
            "POST",
            reply(reply_body),
        ));
        let origin = harness.origin().expect("script origin").to_owned();
        let output = harness
            .settings(json!({"canvasId":CANVAS,"baseUrl":origin}))
            .call("slack.canvas.lookup", json!({"containsText":"Status"}))?;
        assert_ne!(output.status, 0);
        assert!(output.stderr.contains(error), "{}", output.stderr);
        assert_eq!(output.http_calls.len(), 1);
        let request = output.http_request.as_ref().expect("one recorded request");
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.uri,
            format!("{origin}/api/canvases.sections.lookup")
        );
        assert!(
            request
                .headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("content-type")
                    && header.value == b"application/json")
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body)?,
            json!({"canvas_id":CANVAS,"criteria":{"contains_text":"Status"}})
        );
        assert!(
            !request
                .headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("authorization"))
        );
    }
    Ok(())
}
