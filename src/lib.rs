//! Narrow Slack capabilities. No canvas content read or generic API passthrough.

use dekopon_provider_sdk::provider::endpoint::Base;
use dekopon_provider_sdk::provider::{
    Capability, Code, Failure, Http, Proposal, Provider, Settings, Stdout, Usage,
};
use dekopon_provider_sdk::{EffectKind, RiskLevel};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, io::Write};

pub mod commands;
mod slack;

pub struct Slack;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackSettings {
    #[serde(default)]
    canvas_id: Option<String>,
    #[serde(default = "default_base", deserialize_with = "deserialize_base")]
    base_url: Base,
}

impl SlackSettings {
    fn canvas_id(&self) -> Result<&str, SlackError> {
        self.canvas_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or(SlackError::InvalidSettings)
    }
}

fn default_base() -> Base {
    Base::from_static("https://slack.com")
}

fn deserialize_base<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Base, D::Error> {
    let value = String::deserialize(deserializer)?;
    let authority = value
        .strip_prefix("https://")
        .ok_or_else(|| serde::de::Error::custom("invalid Slack base URL"))?;
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    let valid_host = !host.is_empty()
        && host.len() <= 253
        && host.bytes().any(|byte| byte.is_ascii_alphabetic())
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        });
    let valid_port = port.is_none_or(|port| port.parse::<u16>().is_ok_and(|port| port > 0));
    if !valid_host || !valid_port {
        return Err(serde::de::Error::custom("invalid Slack base URL"));
    }
    Base::parse(&value).map_err(|_| serde::de::Error::custom("invalid Slack base URL"))
}

impl Provider for Slack {
    const ID: &'static str = "slack";
    const COMMAND_WORDS: &'static [&'static str] = &["slack"];
    const DESCRIPTION: &'static str = "Narrow Slack canvas and message operations";
    type Args = commands::Args;
    type Capabilities = (Lookup, Replace, Post);

    fn propose(args: Self::Args, _stdin_piped: bool) -> Result<Proposal<Self>, Usage> {
        commands::propose(args)
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LookupInput {
    pub contains_text: String,
}

pub struct Lookup;
impl Capability for Lookup {
    type Provider = Slack;
    const NAME: &'static str = "canvas.lookup";
    const DESCRIPTION: &'static str =
        "Find every matching section ID on the owner-pinned canvas; does not read content";
    const EFFECT: EffectKind = EffectKind::ReadOnly;
    const RISK: RiskLevel = RiskLevel::Low;
    type Input = LookupInput;
    type Needs = (Settings<SlackSettings>, Http);
    type Error = SlackError;

    fn run(
        input: Self::Input,
        (settings, http): Self::Needs,
        out: &mut Stdout,
    ) -> Result<(), Self::Error> {
        let settings = settings.into_inner();
        let canvas_id = settings.canvas_id()?;
        let result = slack::lookup(
            &settings.base_url,
            canvas_id,
            &input.contains_text,
            |request| http.send(request),
        )?;
        serde_json::to_writer(&mut *out, &result).map_err(|_| SlackError::InvalidResponse)?;
        out.write_all(b"\n")
            .map_err(|_| SlackError::InvalidResponse)?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceInput {
    pub section_match: String,
    pub markdown: String,
}

pub struct Replace;
impl Capability for Replace {
    type Provider = Slack;
    const NAME: &'static str = "canvas.replace";
    const DESCRIPTION: &'static str =
        "Replace one freshly looked-up section on the owner-pinned canvas";
    const EFFECT: EffectKind = EffectKind::ExternalWrite;
    const RISK: RiskLevel = RiskLevel::High;
    type Input = ReplaceInput;
    type Needs = (Settings<SlackSettings>, Http);
    type Error = SlackError;

    fn run(
        input: Self::Input,
        (settings, http): Self::Needs,
        out: &mut Stdout,
    ) -> Result<(), Self::Error> {
        let settings = settings.into_inner();
        let result = slack::replace(
            &settings.base_url,
            settings.canvas_id()?,
            &input.section_match,
            &input.markdown,
            |request| http.send(request),
        )?;
        serde_json::to_writer(&mut *out, &result).map_err(|_| SlackError::InvalidResponse)?;
        out.write_all(b"\n")
            .map_err(|_| SlackError::InvalidResponse)?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostInput {
    pub channel: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
}

pub struct Post;
impl Capability for Post {
    type Provider = Slack;
    const NAME: &'static str = "post";
    const DESCRIPTION: &'static str = "Post a message to a channel, optionally in a thread";
    const EFFECT: EffectKind = EffectKind::ExternalWrite;
    const RISK: RiskLevel = RiskLevel::High;
    type Input = PostInput;
    type Needs = (Settings<SlackSettings>, Http);
    type Error = SlackError;

    fn run(
        input: Self::Input,
        (settings, http): Self::Needs,
        out: &mut Stdout,
    ) -> Result<(), Self::Error> {
        let result = slack::post_message(&settings.into_inner().base_url, &input, |request| {
            http.send(request)
        })?;
        serde_json::to_writer(&mut *out, &result).map_err(|_| SlackError::InvalidResponse)?;
        out.write_all(b"\n")
            .map_err(|_| SlackError::InvalidResponse)?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum SlackError {
    InvalidSettings,
    InvalidInput,
    InvalidResponse,
    NotFound,
    Ambiguous,
    Conflict,
    UnknownResult,
    AccessDenied,
    HttpFailed,
    InvalidTimestamp,
    PostRefused,
}
impl fmt::Display for SlackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidSettings => "canvas settings are invalid",
            Self::InvalidInput => "input must have a non-empty value",
            Self::InvalidResponse => "Slack returned an invalid response",
            Self::NotFound => "no matching section or canvas not available",
            Self::Ambiguous => "more than one section matched",
            Self::Conflict => "canvas section edit conflicts with current state",
            Self::UnknownResult => "Slack result could not be verified",
            Self::AccessDenied => "canvas not accessible to this credential",
            Self::HttpFailed => "Slack request failed",
            Self::InvalidTimestamp => "invalid Slack thread timestamp",
            Self::PostRefused => "Slack refused the post",
        })
    }
}
impl Failure for SlackError {
    fn code(&self) -> Code {
        match self {
            Self::InvalidSettings => Code::INVALID_SETTINGS,
            Self::InvalidInput => Code::new("invalid-input"),
            Self::InvalidResponse => Code::new("invalid-response"),
            Self::NotFound => Code::new("not-found"),
            Self::Ambiguous => Code::new("ambiguous"),
            Self::Conflict => Code::new("conflict"),
            Self::UnknownResult => Code::new("unknown-result"),
            Self::AccessDenied => Code::new("access-denied"),
            Self::HttpFailed => Code::new("http-failed"),
            Self::InvalidTimestamp => Code::new("invalid-timestamp"),
            Self::PostRefused => Code::new("post-refused"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SlackError, SlackSettings};
    use dekopon_provider_sdk::provider::{Code, Failure};

    #[test]
    fn empty_canvas_setting_returns_typed_invalid_settings() {
        let settings: SlackSettings =
            serde_json::from_str(r#"{"canvasId":""}"#).expect("settings shape");
        let error = settings.canvas_id().expect_err("empty canvas ID");
        assert!(matches!(error, SlackError::InvalidSettings));
        assert_eq!(error.code(), Code::INVALID_SETTINGS);
    }
}

#[allow(unsafe_code)]
mod export {
    dekopon_provider_sdk::export!(super::Slack);
}
