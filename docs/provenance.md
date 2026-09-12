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

Turnforge's repository license remains Apache-2.0. Cargo dependencies retain
their own licenses. This note records source provenance, not a legal clearance
of the entire dependency graph or future upstream imports.
