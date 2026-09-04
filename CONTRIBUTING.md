# Contributing to Rologlyphex

## Documentation is the authority

This project uses Spec-Driven Development (SDD). Before making a change, read [AGENTS.md](AGENTS.md) and the relevant files in [`sdd/`](sdd/) — especially [`sdd/POLICY.md`](sdd/POLICY.md), which holds the hard development rules (build process, testing requirements, code style, dependency constraints, architecture rules). Changes that conflict with POLICY.md will be asked to change, not merged as-is.

**Pull requests that change behavior must update the relevant `sdd/` documents in the same change** — new features need TDD.md rubrics, architecture changes need TECH.md updates, and so on. See the "Updating SDD documents during maintenance" table in the SDD skill's working guide, or simply: if your PR description explains something that isn't reflected anywhere in `sdd/`, the docs are behind.

## Build pipeline

```bash
cargo clippy --all-targets -- -D warnings
cargo build --release
cargo test
```

All three must pass cleanly. Compiler and clippy warnings are treated as errors — see [`sdd/POLICY.md`](sdd/POLICY.md#build) for the one narrow exception (temporary `dead_code` on shelved features). This is the same pipeline CI runs on every PR. There's no enforced formatter — follow the existing code style (`sdd/POLICY.md`).

## Privacy and attribution

Per [`sdd/POLICY.md`](sdd/POLICY.md#privacy-and-attribution), do not include specific hardware brand names, model numbers, trademarks, or machine-specific identifiers (vendor/product IDs, device configs) anywhere in the repository. Describe hardware generically ("macropad", "secondary keyboard").

## Licensing

Rologlyphex is licensed under [Apache License 2.0](LICENSE). By submitting a contribution, you agree it is licensed under the same terms, per Section 5 of the Apache License, unless you state otherwise explicitly.
