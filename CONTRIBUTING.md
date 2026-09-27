# Contributing to Rizzy

Rizzy is an experimental, source-available port of Fizzy. Before contributing, read the [O'Saasy License](LICENSE.md), including its restriction on directly competing hosted services.

## Report an issue

Include the Rizzy commit, the request or browser steps, the expected Fizzy behavior, and the actual Rizzy behavior. For parity differences, include the pinned Fizzy revision or a link to the relevant upstream test. Remove credentials, tokens, personal data, and private account exports from reports.

## Make a change

1. Open an issue for a substantial change so its scope is clear.
2. Keep the change focused and add a meaningful test when behavior or data migration changes.
3. Run `cargo fmt --check` and `cargo test`. Run the relevant script in `tests/` when your change affects an HTTP route; some scripts need running Rizzy and Fizzy servers.
4. Update [PARITY.md](PARITY.md) for a verified parity change. Do not mark an area fully complete based only on route presence or a single passing example.
5. In the pull request, explain the behavior changed, how you compared it with Fizzy, and any remaining differences.

Local runtime data, the pinned `upstream/` checkout, and Rust build artifacts are ignored by Git. Do not commit secrets, database files, or user exports.
