# Dekopon Slack provider

One Wasm component for separately granted Slack operations. This initial step exposes `slack.canvas.lookup` (`slack canvas lookup TEXT`), returning `{canvasId,sections:[{id}]}` for **all** matching sections on the owner-pinned canvas. It makes one buffered `POST https://slack.com/api/canvases.sections.lookup` with `canvas_id` from `providerSettings.slack.canvasId` and `criteria.contains_text` from the typed input `{containsText}`. No channel or canvas ID is accepted from the caller. Zero matches fail `not-found`; `ok:false` never succeeds. `slack canvas read` explicitly refuses: section IDs are not content.

| Owner-only `providerSettings.slack` | Behaviour |
| --- | --- |
| `canvasId` | Required for canvas calls; absent, null, empty or wrong type fails `invalid-settings` before HTTP. |
| `baseUrl` | Optional; defaults to `https://slack.com`. HTTPS DNS host and optional port only, no path. Invalid values fail `invalid-settings` before HTTP. Omit in production; the component fixture uses `https://slack.com:<ephemeral-port>`. |

Settings are resolved at the authorized call, not broker boot; neither key is model-facing. The guest never sends an authorization header; the broker injects a separately destination-bound credential. No token, scopes, grants or live workspace access are provided by this repository. An offline fixture does not establish Slack bot entitlement or live content preservation.

For the eventual deployment, use the DekoponVille bot token through a broker copy; **the gateway copy also gains canvas-write reach** when those scopes are approved. Ask `canvases:read` for lookup and `canvases:write` for the later replace separately, not file or channel scopes. `slack.post` later uses the existing `chat:write` scope and is independently granted, with no implicit thread fence. Canvas operations require a deployed broker pre-DNS fixed-ID body forbid (method, host, path, no query, `canvas_id`), not merely this setting; the manifest must pin both IDs together. Do not deploy canvas permits before that forbid. Provider HTTP ceilings should follow `gh.yaml`: `slack.com` POST, 10 s, one request, 16,384 request bytes and 262,144 decoded response bytes; replace later overrides three requests and 15 s. No standalone or whole-canvas edit is authorized. Releases, credential copies, bindings, grants and deployment are separate owner actions.

Build and test with the commands in [AGENTS.md](AGENTS.md). The SDK and testkit are both pinned to `=0.46.1`; the component is `slack-provider.wasm`. Tests use synthetic decoded JSON section IDs and the owner-selected canvas ID, not recorded Slack traffic or proof of the broker forbid.
