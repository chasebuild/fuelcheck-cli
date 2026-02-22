# Contributing

Thanks for helping improve Fuelcheck CLI. This guide covers the local dev setup, style, and test expectations.

**Prerequisites**
- Rust stable with 2024 edition support.
- Standard Rust tooling (`cargo`, `rustfmt`, `clippy`).

**Local Setup**
Build the binary:
```bash
cargo build --workspace
```

Run the CLI in dev mode:
```bash
cargo run -p fuelcheck-cli -- --help
```

**Development Workflow**
- Keep changes focused and scoped to a single goal.
- Add or update tests when behavior changes.
- Avoid committing secrets or real credentials.

**Style**
Format and lint before submitting:
```bash
cargo fmt --all
cargo clippy --all-targets --all-features
```

**Tests**
Run the test suite:
```bash
cargo test --workspace
```
If you add provider integrations that require live credentials, keep tests unit-level and avoid network calls where possible.

**Adding a Provider**
Checklist for a new provider implementation:
- Add a new provider module in `core/src/providers/`.
- Update `ProviderId` and `ProviderSelector` in `core/src/providers/mod.rs`.
- Register the provider in `ProviderRegistry::new` in `core/src/providers/mod.rs`.
- Add any required config fields to `core/src/config.rs` if needed.
- Ensure `fuelcheck-cli usage --provider <id>` works for the new provider.

**Reporting Issues**
Please include:
- The exact command you ran.
- The output (redact tokens and personal data).
- Your OS and Rust version.

**Security**
Do not share tokens, cookies, or API keys. Redact logs and config files before posting.
