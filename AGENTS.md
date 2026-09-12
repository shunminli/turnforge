# Turnforge repository guidance

## Documentation-first maintenance

- Read [docs/README.md](docs/README.md), then the architecture and design documents for the module being changed.
- Every stable core module has separate `architecture.md` and `design.md` under `docs/modules/<module>/`.
- Follow [the documentation guide](docs/documentation-guide.md): update current contracts when interfaces, ownership, cancellation, errors, permissions, or limits change. Update the module map when boundaries change.
- Document implemented behavior separately from proposals. Do not claim tests enforce a contract unless the cited check exists.
- Documentation is repository guidance, not a runtime policy or an automatic CI enforcement mechanism.

## Scope and verification

- Preserve unrelated/uncommitted work. Do not commit, push or publish without authorization.
- Keep the core headless; terminal, configuration and process signal handling belong to the host.
- Use code and tests as the evidence for current behavior. Do not modify runtime behavior solely to make an explanatory document true.
- For runtime changes, run the relevant tests and the checks in [.github/workflows/ci.yml](.github/workflows/ci.yml).
- For documentation-only changes, verify relative links, module coverage and claim accuracy; do not claim live-provider or remote-CI validation from local checks.
