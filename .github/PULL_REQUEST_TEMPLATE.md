## Change

<!-- The problem, what changes for the user, and the scope. -->

Related issue:

## Checks

<!-- Paste the commands you ran and their result. List anything you couldn't check. -->

- [ ] `pnpm run lint`, `pnpm exec vue-tsc --noEmit`, `pnpm run test:unit` and `pnpm run build` pass.
- [ ] Rust changes: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass in `src-tauri/`.
- [ ] A fix comes with a test that fails without it.
- [ ] UI changes: strings in `en.json`, `ar.json` and `fr.json`; Arabic checked right-to-left; light and dark checked; `pnpm run test:e2e` run for changed flows.
- [ ] Works with default settings; new power options sit under Advanced.
- [ ] No secrets, cookies, real user links, personal paths or raw logs in the code, tests or screenshots.
- [ ] Nothing new is sent over the network without an opt-in that is off by default.
