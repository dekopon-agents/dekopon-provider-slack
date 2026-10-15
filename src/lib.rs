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
    canvas_id: String,
    #[serde(default = "default_base", deserialize_with = "deserialize_base")]
    base_url: Base,
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
    const DESCRIPTION: &'static str = "Narrow Slack canvas section lookup";
    type Args = commands::Args;
    type Capabilities = (Lookup,);

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
        let canvas_id = settings.canvas_id;
        if canvas_id.trim().is_empty() {
            return Err(SlackError::InvalidSettings);
        }
        let result = slack::lookup(
            &settings.base_url,
            &canvas_id,
            &input.contains_text,
            |request| http.send(request),
        )?;
        serde_json::to_writer(&mut *out, &result).map_err(|_| SlackError::InvalidResponse)?;
        out.write_all(b"\n")
            .map_err(|_| SlackError::InvalidResponse)?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum SlackError {
    InvalidSettings,
    InvalidResponse,
    NotFound,
    AccessDenied,
    HttpFailed,
}
impl fmt::Display for SlackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidSettings => "canvas settings are invalid",
            Self::InvalidResponse => "Slack returned an invalid response",
            Self::NotFound => "no matching section or canvas not available",
            Self::AccessDenied => "canvas not accessible to this credential",
            Self::HttpFailed => "Slack request failed",
        })
    }
}
impl Failure for SlackError {
    fn code(&self) -> Code {
        match self {
            Self::InvalidSettings => Code::INVALID_SETTINGS,
            Self::InvalidResponse => Code::new("invalid-response"),
            Self::NotFound => Code::new("not-found"),
            Self::AccessDenied => Code::new("access-denied"),
            Self::HttpFailed => Code::new("http-failed"),
        }
    }
}

#[allow(unsafe_code)]
mod export {
    dekopon_provider_sdk::export!(super::Slack);
}
