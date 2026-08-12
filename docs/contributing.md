# Contributing to OpenHeart

Thank you for contributing to **OpenHeart**! Created and maintained by **Ahmad Hassan (B-Ted)**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## Development Guidelines

1. **Branching Strategy**: Create feature branches off `main`:
   ```bash
   git checkout -b feat/your-feature-name
   ```

2. **Code Standards**:
   - Maintain zero compilation warnings across all targets.
   - Enforce 100% passing tests for all submodules in `src/core/` and `src/phase1/`.
   - Preserve memory layout invariants (`TokenRecord` 16B, `SourceFileRecord` 64B, `sort_key` packing).

3. **Commit Messages**: Use Conventional Commits format:
   ```bash
   git commit -m "feat: add tree-sitter parser adapter for Kotlin"
   ```

4. **Pull Requests**:
   - Run `./scripts/ci_check.sh` prior to submitting your PR.
   - Update relevant documentation under `docs/` or `README.md`.
