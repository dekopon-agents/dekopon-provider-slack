# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-06

### Added

- `slack.canvas.read` (`slack canvas read`) returns the whole owner-pinned canvas as `{canvasId,markdown}`, granted separately from lookup, replace and post. A canvas over the broker's response ceiling is refused by the broker as a byte-limit host rejection, never returned in part, and the provider maps a host response-too-large to `too-large`.

## [0.1.0] - 2026-10-15

### Added

- First release: separately granted `slack.canvas.lookup` and `slack.canvas.replace` against the owner-pinned canvas from `providerSettings.slack.canvasId`, plus separately granted `slack.post` to a caller-supplied channel, optionally in a thread.

### Changed

- A canvas edit or post whose request was refused before it was sent (broker denial, request or size limit, DNS, connect, TLS) fails `http-failed`; `unknown-result` is reserved for timeout, protocol, oversized-response and internal failures after a send.
- An empty canvas lookup query, section match or Markdown fails `invalid-input` with exit status 2 before any HTTP request, instead of `not-found` or exit 1.
- A verification lookup that fails after an accepted canvas edit reports `unknown-result`, since the edit may already have applied.
