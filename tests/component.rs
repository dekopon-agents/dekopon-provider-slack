use dekopon_provider_sdk::provider::Response;
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
fn manifest_declares_separate_canvas_capabilities() -> Result<(), Box<dyn std::error::Error>> {
    conformance::<Slack>(component())?;
    let manifest = provider::manifest::<Slack>()?;
    assert_eq!(manifest.capabilities.len(), 3);
    assert_eq!(manifest.capabilities[2].id.as_str(), "slack.post");
    assert_eq!(manifest.capabilities[1].id.as_str(), "slack.canvas.replace");
    assert_eq!(manifest.capabilities[0].id.as_str(), "slack.canvas.lookup");
    assert_eq!(
        manifest.capabilities[0].input_schema["additionalProperties"],
        false
    );
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
    let CommandRunOutcome::Proposed {
        capability, input, ..
    } = provider::command::<Slack>(
        &[
            "canvas".into(),
            "replace".into(),
            "Status".into(),
            "New".into(),
        ],
        false,
    )
    else {
        panic!("replace must propose")
    };
    assert_eq!(capability.as_str(), "slack.canvas.replace");
    assert_eq!(input, json!({"sectionMatch":"Status","markdown":"New"}));
    assert!(matches!(
        provider::command::<Slack>(&["canvas".into(), "read".into()], false),
        CommandRunOutcome::Failed { .. }
    ));
}

#[test]
fn post_proposes_typed_top_level_and_thread_inputs() {
    for (argv, expected) in [
        (
            vec!["post", "C123", "hello"],
            json!({"channel":"C123","text":"hello"}),
        ),
        (
            vec!["post", "C123", "hello", "123.000001"],
            json!({"channel":"C123","text":"hello","threadTs":"123.000001"}),
        ),
    ] {
        let CommandRunOutcome::Proposed {
            capability, input, ..
        } = provider::command::<Slack>(
            &argv.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            false,
        )
        else {
            panic!("post must propose")
        };
        assert_eq!(capability.as_str(), "slack.post");
        assert_eq!(input, expected);
    }
}

#[test]
fn post_component_sends_one_bounded_message_without_canvas_settings()
-> Result<(), Box<dyn std::error::Error>> {
    for (input, expected_body) in [
        (
            json!({"channel":"C123","text":"hello"}),
            json!({"channel":"C123","text":"hello"}),
        ),
        (
            json!({"channel":"C123","text":"hello","threadTs":"123.000001"}),
            json!({"channel":"C123","text":"hello","thread_ts":"123.000001"}),
        ),
    ] {
        let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
            "slack.com",
            "POST",
            reply(json!({"ok":true,"channel":"C123","ts":"124.000002"})),
        ));
        let origin = harness.origin().expect("script origin").to_owned();
        let output = harness
            .settings(json!({"baseUrl":origin}))
            .call("slack.post", input)?;
        assert_eq!(output.status, 0, "{}", output.stderr);
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout)?,
            json!({"channel":"C123","ts":"124.000002"})
        );
        assert_eq!(output.http_calls.len(), 1);
        let request = output.http_request.as_ref().expect("one recorded request");
        assert_eq!(request.uri, format!("{origin}/api/chat.postMessage"));
        assert_eq!(request.method, "POST");
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body)?,
            expected_body
        );
        assert!(
            !request
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("authorization"))
        );
    }
    Ok(())
}

#[test]
fn post_refusal_and_uncertain_response() -> Result<(), Box<dyn std::error::Error>> {
    for (body, code) in [
        (
            json!({"ok":false,"error":"not_in_channel"}),
            "Slack refused the post",
        ),
        (
            json!({"ok":true,"channel":"C123","ts":"broken"}),
            "Slack result could not be verified",
        ),
        (
            json!({"ok":true,"channel":"C123"}),
            "Slack result could not be verified",
        ),
    ] {
        let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
            "slack.com",
            "POST",
            reply(body),
        ));
        let origin = harness.origin().expect("script origin").to_owned();
        let output = harness
            .settings(json!({"baseUrl":origin}))
            .call("slack.post", json!({"channel":"C123","text":"hello"}))?;
        assert_eq!(output.status, 1);
        assert!(output.stderr.contains(code), "{}", output.stderr);
        assert_eq!(output.http_calls.len(), 1);
        let request = output.http_request.as_ref().expect("one request");
        assert_eq!(request.uri, format!("{origin}/api/chat.postMessage"));
        assert_eq!(request.method, "POST");
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body)?,
            json!({"channel":"C123","text":"hello"})
        );
        assert!(
            !request
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("authorization"))
        );
    }
    Ok(())
}

#[test]
fn native_post_requires_no_canvas_setting() -> Result<(), Box<dyn std::error::Error>> {
    let native = Native::<Slack>::new().http(HttpScript::new(
        "slack.com",
        "POST",
        reply(json!({"ok":true,"channel":"C123","ts":"124.000002"})),
    ));
    let output = native.call(
        "slack.post",
        &json!({"channel":"C123","text":"hello"}).to_string(),
    );
    assert_eq!(output.status, 0, "{}", output.stderr);
    let requests = native.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].uri, "https://slack.com/api/chat.postMessage");
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body)?,
        json!({"channel":"C123","text":"hello"})
    );
    assert!(
        !requests[0]
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("authorization"))
    );
    Ok(())
}

#[test]
fn malformed_thread_timestamp_refuses_before_http() -> Result<(), Box<dyn std::error::Error>> {
    let output = Harness::<Slack>::get(component()).call(
        "slack.post",
        json!({"channel":"C123","text":"hello","threadTs":"bad"}),
    )?;
    assert_eq!(output.status, 1);
    assert!(
        output.stderr.contains("invalid Slack thread timestamp"),
        "{}",
        output.stderr
    );
    assert!(output.http_calls.is_empty());
    Ok(())
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
fn empty_lookup_query_and_replace_match_refuse_before_http()
-> Result<(), Box<dyn std::error::Error>> {
    for (capability, input) in [
        ("slack.canvas.lookup", json!({"containsText":"  "})),
        (
            "slack.canvas.replace",
            json!({"sectionMatch":"  ","markdown":"New"}),
        ),
    ] {
        let output = Harness::<Slack>::get(component())
            .settings(json!({"canvasId":CANVAS}))
            .call(capability, input)?;
        assert_eq!(output.status, 2, "{}", output.stderr);
        assert!(
            output.stderr.contains("non-empty value"),
            "{}",
            output.stderr
        );
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

#[test]
fn replace_refuses_before_a_second_request() -> Result<(), Box<dyn std::error::Error>> {
    for (settings, markdown, body, expected_calls, status, message) in [
        (
            json!({"canvasId":null}),
            "New",
            json!({"ok":true,"sections":[{"id":"one"}]}),
            0,
            1,
            "settings",
        ),
        (
            json!({"canvasId":CANVAS}),
            " \n\t",
            json!({"ok":true,"sections":[{"id":"one"}]}),
            0,
            2,
            "non-empty",
        ),
        (
            json!({"canvasId":CANVAS}),
            "New",
            json!({"ok":true,"sections":[]}),
            1,
            1,
            "no matching",
        ),
        (
            json!({"canvasId":CANVAS}),
            "New",
            json!({"ok":true,"sections":[{"id":"one"},{"id":"two"}]}),
            1,
            1,
            "more than one",
        ),
        (
            json!({"canvasId":CANVAS}),
            "New",
            json!({"ok":false,"sections":[{"id":"one"}]}),
            1,
            1,
            "request failed",
        ),
    ] {
        let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
            "slack.com",
            "POST",
            reply(body),
        ));
        let origin = harness.origin().expect("script origin").to_owned();
        let mut settings = settings;
        settings["baseUrl"] = json!(origin);
        let output = harness.settings(settings).call(
            "slack.canvas.replace",
            json!({"sectionMatch":"Status","markdown":markdown}),
        )?;
        assert_eq!(output.status, status);
        assert!(output.stderr.contains(message), "{}", output.stderr);
        assert_eq!(output.http_calls.len(), expected_calls);
        if expected_calls == 1 {
            let request = output.http_request.as_ref().expect("lookup request");
            assert_eq!(
                request.uri,
                format!("{origin}/api/canvases.sections.lookup")
            );
            assert_eq!(request.method, "POST");
            assert_eq!(
                serde_json::from_slice::<Value>(&request.body)?,
                json!({"canvas_id":CANVAS,"criteria":{"contains_text":"Status"}})
            );
            assert!(
                !request
                    .headers
                    .iter()
                    .any(|h| h.name.eq_ignore_ascii_case("authorization"))
            );
        }
    }
    Ok(())
}

#[test]
fn native_replace_sends_one_section_edit_and_one_verification()
-> Result<(), Box<dyn std::error::Error>> {
    let native = Native::<Slack>::new()
        .settings(json!({"canvasId":CANVAS}))
        .http(HttpScript::new(
            "slack.com",
            "POST",
            reply(json!({"ok":true,"sections":[{"id":"one"}]})),
        ));
    let markdown = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789--EXTRA\nnext";
    let output = native.call(
        "slack.canvas.replace",
        &json!({"sectionMatch":"Status","markdown":markdown}).to_string(),
    );
    assert_eq!(output.status, 0, "{}", output.stderr);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?,
        json!({"canvasId":CANVAS,"sectionId":"one"})
    );
    let requests = native.requests();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        assert_eq!(request.method, "POST");
        assert!(
            !request
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("authorization"))
        );
    }
    assert_eq!(
        requests[0].uri,
        "https://slack.com/api/canvases.sections.lookup"
    );
    assert_eq!(requests[1].uri, "https://slack.com/api/canvases.edit");
    assert_eq!(
        requests[2].uri,
        "https://slack.com/api/canvases.sections.lookup"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body)?,
        json!({"canvas_id":CANVAS,"criteria":{"contains_text":"Status"}})
    );
    let edit: Value = serde_json::from_slice(&requests[1].body)?;
    assert_eq!(
        edit,
        json!({"canvas_id":CANVAS,"changes":[{"operation":"replace","section_id":"one","document_content":{"type":"markdown","markdown":markdown}}]})
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[2].body)?,
        json!({"canvas_id":CANVAS,"criteria":{"contains_text":"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789--"}})
    );
    Ok(())
}
