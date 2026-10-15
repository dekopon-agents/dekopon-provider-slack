# Guidance for coding agents

The Slack provider is a Wasm component with separately granted capabilities. Read [README.md](README.md) before changes. Core's [boundaries](https://github.com/dekopon-agents/dekopon/blob/main/AGENTS.md#boundaries-that-must-survive), [proportionate remedies](https://github.com/dekopon-agents/dekopon/blob/main/AGENTS.md#proportionate-remedies) and [Rust guidelines](https://github.com/dekopon-agents/dekopon/blob/main/AGENTS.md#rust-guidelines) apply. Tags, releases and version bumps need owner authorization; a PR never pushes main.

## Boundaries

- The guest never sets authorization; the broker injects destination-bound credentials at the native HTTP boundary. Never put token bytes in input, output or errors.
- The canvas ID and optional HTTPS DNS-only `baseUrl` are owner settings, not model input. Production omits `baseUrl`. A grant to lookup is not a grant to replace or post. Content read is held. No generic Slack API passthrough.
- A canvas write must name one freshly looked-up section ID. No standalone or whole-canvas edits. Provider code alone is not the fixed-ID broker fence.

## Build and verify

A PR passes `ci / validate` from [provider-workflows](https://github.com/dekopon-agents/provider-workflows). Run its steps and the local rustdoc gate before a push:

```console
git clone --depth 1 https://github.com/dekopon-agents/provider-workflows ../provider-workflows
cargo fmt --all --check
cargo deny --all-features check bans licenses sources advisories
cargo clippy --locked -p dekopon-slack-provider --all-targets -- -D warnings
cargo clippy --locked -p dekopon-slack-provider --lib --target wasm32-unknown-unknown -- -D warnings
../provider-workflows/build.sh
DEKOPON_PROVIDER_COMPONENT="$PWD/slack-provider.wasm" cargo test --locked -p dekopon-slack-provider
```

Local gate (not part of `ci / validate`): `RUSTDOCFLAGS="-D warnings" cargo doc --locked -p dekopon-slack-provider --no-deps`.

Pins are exact (`rust-toolchain.toml`, wasm-tools 1.259.0); build.sh asserts reproducible bytes. Tests use offline decoded JSON fixtures, not live Slack.

## Change rules

- No comments by default except useful help text. Test names state invariants.
- Component tests read `DEKOPON_PROVIDER_COMPONENT` with `expect`, without skip or fallback. A native-only test says native in its name.
- Every request test asserts full URI, count, method, body and absence of guest authorization. Name what `HttpScript` observes; no fabricated cassette.
- Update README for capability or setting changes. No CHANGELOG.md in feature PRs; report user-visible changes in the PR body.
- Move SDK and testkit pins together exactly and regenerate Cargo.lock with Cargo.
