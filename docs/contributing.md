# Contributing to OpenHeart

Thank you for contributing to **OpenHeart**! Created and maintained by **Ahmad Hassan (B-Ted)**. Co-maintainers, reviewers and first-time contributors are all welcome; questions and design discussions belong in [GitHub Discussions](https://github.com/AhmadHassan-BTed/OpenHeart/discussions).

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## Development Guidelines

1. **Toolchain**: Rust **1.88** or newer (`rust-version` in `Cargo.toml`). `make msrv` checks the crate on exactly that version.

2. **Branching Strategy**: Create feature branches off `main`:
   ```bash
   git checkout -b feat/your-feature-name
   ```

3. **Quality Gates** (run them before opening a pull request):
   ```bash
   make ci        # rustfmt check, cargo check, clippy -D warnings, tests, doctests, rustdoc -D warnings
   make ci-full   # the above plus MSRV, cargo-deny and cargo-machete
   ```
   - Zero compiler and clippy warnings across all targets.
   - All tests pass, including doctests (`cargo test --all-targets` alone skips doctests; `make test` runs both).
   - Changes to code that uses `unsafe` also pass `make miri`.
   - Preserve the documented binary layouts (`TokenRecord` 16 B, `SourceFileRecord` 64 B, `sort_key` packing); layout tests live beside the types in `src/core/types/`.

4. **Tests assert expected results.** A test must compare output against an expected value — a hand-checked fixture, a golden file, or an independent oracle. Tests that only check for non-empty output or absence of a panic do not count as evidence for a feature.

5. **Commit Messages**: Use the Conventional Commits format:
   ```bash
   git commit -m "feat(ingestion): add tree-sitter-kotlin grammar adapter"
   ```

6. **Pull Requests**:
   - Update the relevant documentation under `docs/` or `README.md` in the same pull request.
   - **Status rule**: if the change implements, removes or alters a capability, update its row in [`docs/status.md`](./status.md) and its item in [`ROADMAP.md`](../ROADMAP.md). An item may be marked implemented only when the row links to the module **and** to a test that asserts an expected result.
   - **Numbers rule**: do not add accuracy, size or speed figures to any document unless they come from the [measurement protocol](./status.md#7-measurement-protocol) with committed result files.
   - **Configuration rule**: a new environment variable is documented in [`docs/status.md` §8](./status.md#8-configuration-actually-read-by-the-engine) and `.env.example` in the same change that makes the code read it.

---

## Where to Start

- Rows marked **Partial**, **Template**, **Not derived** or **Planned** in [`docs/status.md`](./status.md) are the open work, each with the module to change.
- The [measurement protocol](./status.md#7-measurement-protocol) lists benchmark suites that have no harness yet; building any one of them is a self-contained contribution.
- Architectural decisions and their trade-offs are recorded in [`technical-decisions.md`](./technical-decisions.md); propose a new ADR there for any change that alters one.
