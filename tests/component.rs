use dekopon_broker_host::BrokerHostError;
use dekopon_provider_sdk::provider::{Request, Response};
use dekopon_provider_sdk::{CommandRunOutcome, EffectKind, provider};
use dekopon_provider_sdk_testkit::{Harness, HarnessError, HttpScript, Native, conformance};
use dekopon_slack_provider::Slack;
use serde_json::{Value, json};

const CANVAS: &str = "F0SYNTHCANVAS";

fn component() -> std::path::PathBuf {
    std::env::var_os("DEKOPON_PROVIDER_COMPONENT")
        .expect("DEKOPON_PROVIDER_COMPONENT must name the freshly built component")
        .into()
}

fn assert_read_request(request: &Request, origin: &str) -> Result<(), serde_json::Error> {
    assert_eq!(request.uri, format!("{origin}/api/canvases.getContent"));
    assert_eq!(request.method, "POST");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({"canvas_id":CANVAS,"content_type":"markdown"})
    );
    assert!(
        !request
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("authorization"))
    );
    Ok(())
}

fn reply(value: Value) -> Response {
    Response {
        status: 200,
        headers: vec![],
        body: value.to_string().into_bytes(),
    }
}

#[test]
fn manifest_declares_four_separate_capabilities_with_a_closed_read_only_read()
-> Result<(), Box<dyn std::error::Error>> {
    conformance::<Slack>(component())?;
    let manifest = provider::manifest::<Slack>()?;
    assert_eq!(manifest.capabilities.len(), 4);
    let read = &manifest.capabilities[3];
    assert_eq!(read.id.as_str(), "slack.canvas.read");
    assert_eq!(read.effect, EffectKind::ReadOnly);
    assert_eq!(read.input_schema["type"], "object");
    assert_eq!(read.input_schema["additionalProperties"], false);
    assert!(
        read.input_schema
            .get("properties")
            .is_none_or(|properties| properties == &json!({}))
    );
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
fn native_canvas_verbs_propose_typed_inputs_and_read_takes_no_arguments() {
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
    let CommandRunOutcome::Proposed {
        capability, input, ..
    } = provider::command::<Slack>(&["canvas".into(), "read".into()], false)
    else {
        panic!("read must propose")
    };
    assert_eq!(capability.as_str(), "slack.canvas.read");
    assert_eq!(input, json!({}));
    let CommandRunOutcome::Rendered { stdout, status, .. } = provider::command::<Slack>(
        &["canvas".into(), "read".into(), "temp:C:one".into()],
        false,
    ) else {
        panic!("read with an argument must be a usage rejection")
    };
    assert_eq!(status, 2);
    assert!(stdout.is_empty());
}

#[test]
fn native_post_proposes_typed_top_level_and_thread_inputs() {
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
    for (body, message) in [
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
        assert_eq!(output.stderr, format!("{message}\n"));
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
fn native_single_block_replace_sends_three_requests_and_verifies_with_the_replacement_text()
-> Result<(), Box<dyn std::error::Error>> {
    let native = Native::<Slack>::new()
        .settings(json!({"canvasId":CANVAS}))
        .http(HttpScript::new(
            "slack.com",
            "POST",
            reply(json!({"ok":true,"sections":[{"id":"one"}]})),
        ));
    let markdown = "\n ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789--EXTRA \n";
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

#[test]
fn replace_refuses_multi_block_markdown_before_http_without_echoing_it()
-> Result<(), Box<dyn std::error::Error>> {
    for markdown in [
        "## Log\n\nLog line one: synthetic canvas created.\n\nmac-verify after 2026-11-07",
        "## Log\nmac-verify after 2026-11-07",
        "- first\n- second",
    ] {
        let output = Harness::<Slack>::get(component())
            .settings(json!({"canvasId":CANVAS}))
            .call(
                "slack.canvas.replace",
                json!({"sectionMatch":"mac-verify before","markdown":markdown}),
            )?;
        assert_eq!(output.status, 2, "{}", output.stderr);
        assert_eq!(
            output.stderr,
            "Markdown must be one block: one non-empty line, with no heading or other block beside it\n"
        );
        assert!(!output.stderr.contains("mac-verify"));
        assert!(output.stdout.is_empty());
        assert!(output.http_calls.is_empty());
    }
    Ok(())
}

#[test]
fn section_id_as_section_match_is_searched_as_text_and_refused_not_found_after_one_lookup()
-> Result<(), Box<dyn std::error::Error>> {
    let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
        "slack.com",
        "POST",
        reply(json!({"ok":true,"sections":[]})),
    ));
    let origin = harness.origin().expect("script origin").to_owned();
    let output = harness
        .settings(json!({"canvasId":CANVAS,"baseUrl":origin}))
        .call(
            "slack.canvas.replace",
            json!({"sectionMatch":"temp:C:synthetic0001","markdown":"New"}),
        )?;
    assert_eq!(output.status, 1);
    assert_eq!(
        output.stderr,
        "no matching section or canvas not available\n"
    );
    assert_eq!(output.http_calls.len(), 1);
    let request = output.http_request.as_ref().expect("lookup request");
    assert_eq!(
        request.uri,
        format!("{origin}/api/canvases.sections.lookup")
    );
    assert_eq!(request.method, "POST");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({"canvas_id":CANVAS,"criteria":{"contains_text":"temp:C:synthetic0001"}})
    );
    assert!(
        !request
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("authorization"))
    );
    Ok(())
}

#[test]
fn native_canvas_help_says_section_match_is_text_markdown_is_one_block_and_lookup_ids_are_not_replace_input()
 {
    let help = |verb: &str| {
        let CommandRunOutcome::Rendered { stdout, status, .. } =
            provider::command::<Slack>(&["canvas".into(), verb.into(), "--help".into()], false)
        else {
            panic!("help must render")
        };
        assert_eq!(status, 0);
        stdout
    };
    let replace = help("replace");
    for phrase in [
        "Text that must appear in exactly one block of the canvas; it is search text, not a section ID",
        "replaces only the matched block; carry only that block's new text, never its surrounding heading or neighbouring blocks",
        "After any failure, run `slack canvas read` before another edit.",
    ] {
        assert!(replace.contains(phrase), "{replace}");
    }
    let lookup = help("lookup");
    assert!(
        lookup.contains("the IDs are for information and are not replace input"),
        "{lookup}"
    );
}

#[test]
fn read_sends_one_markdown_get_content_for_the_pinned_canvas_and_writes_its_markdown()
-> Result<(), Box<dyn std::error::Error>> {
    let markdown = "# Plan\n## Status\nStatus: amber, synthetic.";
    let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
        "slack.com",
        "POST",
        reply(json!({"ok":true,"content":markdown})),
    ));
    let origin = harness.origin().expect("script origin").to_owned();
    let output = harness
        .settings(json!({"canvasId":CANVAS,"baseUrl":origin}))
        .call("slack.canvas.read", json!({}))?;
    assert_eq!(output.status, 0, "{}", output.stderr);
    assert_eq!(
        output.stdout,
        format!("{}\n", json!({"canvasId":CANVAS,"markdown":markdown})).into_bytes()
    );
    assert_eq!(output.http_calls.len(), 1);
    assert_read_request(output.http_request.as_ref().expect("one request"), &origin)?;
    Ok(())
}

#[test]
fn read_refusals_fail_with_their_message_empty_stdout_and_one_request()
-> Result<(), Box<dyn std::error::Error>> {
    for (status, body, message) in [
        (
            500,
            json!({"ok":true,"content":"x"}).to_string(),
            "Slack request failed",
        ),
        (
            200,
            json!({"ok":false,"error":"ratelimited"}).to_string(),
            "Slack request failed",
        ),
        (
            200,
            json!({"ok":false,"error":"canvas_not_found"}).to_string(),
            "no matching section or canvas not available",
        ),
        (
            200,
            json!({"ok":false,"error":"canvas_deleted"}).to_string(),
            "no matching section or canvas not available",
        ),
        (
            200,
            json!({"ok":false,"error":"access_denied"}).to_string(),
            "canvas not accessible to this credential",
        ),
        (
            200,
            json!({"ok":false,"error":"missing_scope"}).to_string(),
            "canvas not accessible to this credential",
        ),
        (
            200,
            json!({"ok":false,"error":"canvas_disabled_user_team"}).to_string(),
            "canvas not accessible to this credential",
        ),
        (
            200,
            "not json".to_owned(),
            "Slack returned an invalid response",
        ),
        (
            200,
            json!({"ok":true}).to_string(),
            "Slack returned an invalid response",
        ),
    ] {
        let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
            "slack.com",
            "POST",
            Response {
                status,
                headers: vec![],
                body: body.into_bytes(),
            },
        ));
        let origin = harness.origin().expect("script origin").to_owned();
        let output = harness
            .settings(json!({"canvasId":CANVAS,"baseUrl":origin}))
            .call("slack.canvas.read", json!({}))?;
        assert_eq!(output.status, 1);
        assert_eq!(output.stderr, format!("{message}\n"));
        assert!(output.stdout.is_empty());
        assert_eq!(output.http_calls.len(), 1);
        assert_read_request(output.http_request.as_ref().expect("one request"), &origin)?;
    }
    Ok(())
}

#[test]
fn read_over_the_harness_response_cap_is_rejected_by_the_host_not_returned()
-> Result<(), Box<dyn std::error::Error>> {
    let content = "a".repeat(70 * 1024);
    let harness = Harness::<Slack>::get(component()).http(HttpScript::new(
        "slack.com",
        "POST",
        reply(json!({"ok":true,"content":content})),
    ));
    let origin = harness.origin().expect("script origin").to_owned();
    let result = harness
        .settings(json!({"canvasId":CANVAS,"baseUrl":origin}))
        .call("slack.canvas.read", json!({}));
    let Err(HarnessError::Invocation(failure)) = result else {
        panic!("an over-cap read must be a host rejection")
    };
    let BrokerHostError::HostCallRejected {
        capability, reason, ..
    } = *failure.error
    else {
        panic!("an over-cap read must be HostCallRejected")
    };
    assert_eq!(capability.as_str(), "slack.canvas.read");
    assert_eq!(reason, "byte-limit");
    assert_eq!(failure.http_calls.len(), 1);
    Ok(())
}

#[test]
fn read_without_a_canvas_setting_refuses_before_http() -> Result<(), Box<dyn std::error::Error>> {
    for settings in [
        None,
        Some(json!({"canvasId":null})),
        Some(json!({"canvasId":""})),
    ] {
        let mut harness = Harness::<Slack>::get(component());
        if let Some(settings) = settings {
            harness = harness.settings(settings);
        }
        let output = harness.call("slack.canvas.read", json!({}))?;
        assert_eq!(output.status, 1);
        assert_eq!(output.stderr, "canvas settings are invalid\n");
        assert!(output.stdout.is_empty());
        assert!(output.http_calls.is_empty());
    }
    Ok(())
}
