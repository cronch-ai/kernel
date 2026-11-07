# Contributing

Fixes and improvements are welcome, from people and from agents. A pull request written by an agent is fine: it's reviewed like any other, and a proof is accepted because it checks, not because of who wrote it.

## Before you start

For anything bigger than a fix, open an issue first, so we can agree on the approach before you write it.

## Building and testing

```sh
npm ci
cargo test --workspace
npm run build                  # javascript/kernel.wasm
npm test --prefix javascript
npm run lint
```

The proof tests need cvc5, Ethos and Lean: `scripts/install-proof-tools.sh` and `scripts/install-lean.sh` install them, then `npm run test:proofs`.

See the [README](README.md#development) for more. The dev container provides every tool you need.

## Pull requests

- One change per pull request.
- Commit messages are a single line, in [Conventional Commits](https://www.conventionalcommits.org/) form, as short as they can be: `feat: add Foo`, `fix(run): drop a stale grant`.
- Run the tests and `npm run lint` before you push. CI runs them too.

By contributing, you agree that your contribution is licensed under the [Apache License 2.0](LICENSE), as the rest of the repo is.

## Security

Don't report a vulnerability in an issue. See [SECURITY.md](SECURITY.md).
