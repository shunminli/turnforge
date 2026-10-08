# Provenance

Turnforge is a new Rust implementation informed by the design and observable
behavior of [Helixent](https://github.com/magiccube/helixent), by Henry Lin
(`package.json` author metadata).

Reference inspected: version 1.3.1, commit
`5cc1fb3faf29b8db8dce614e925c89030ae2558b`.
Primary reference areas: foundation message/model/tool contracts, agent loop,
OpenAI stream handling, file tools and shell execution.

This milestone does not vendor Helixent source, assets, or tests. Rust modules
and tests were newly authored around the contracts described in
`helixent-migration.md`, with intentional changes to state ownership,
execution ordering, path rules and cancellation. There is no affiliation claim.

Helixent's inspected `package.json` declares MIT; no tracked standalone LICENSE,
NOTICE or COPYING file was found in that checkout. Do not infer an upstream
copyright notice or relicense future copied material based on the manifest alone.
Before importing/translating concrete upstream code or assets, verify the
applicable grant and retain the required original notices alongside the material.

## OpenCoder reliability ideas (2026-10-08)

Reference inspected: [MoSunDay/opencoder](https://github.com/MoSunDay/opencoder),
commit `8bf74a10109dc16c0d087df23e1ea829ed1dd259` (fork synchronized at
`shunminli/opencoder` merge `1b3b04d85b7f5d6864c79ead84fcad7cb3c738db`).
Primary reference areas: `crates/session/src/runner/mod.rs` loop detection and
`crates/llm/src/retry.rs` / `client/transport.rs` request retries.

The new Turnforge implementation and tests were authored independently; no
OpenCoder code, tests or assets were vendored. Intentional differences: compare
consecutive whole tool batches rather than upstream's recent per-call window;
retry only explicit HTTP 429/502/503/504 before a successful stream, never
transport failures or interrupted streams. Both are opt-in and retain the
existing single-writer transcript, serial tools and cancellation lifecycle.

The inspected OpenCoder LICENSE grants MIT with `Copyright (c) 2026 MoSunDay`.
Any future concrete copy must retain that notice and grant; this conceptual
adaptation does not claim affiliation or a blanket clearance for future imports.

Turnforge's repository license remains Apache-2.0. Cargo dependencies retain
their own licenses. This note records source provenance, not a legal clearance
of the entire dependency graph or future upstream imports.
