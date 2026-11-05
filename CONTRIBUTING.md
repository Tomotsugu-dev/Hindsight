# Contributing to Hindsight

Thank you for helping improve Hindsight. Hindsight records sensitive activity
data and runs on Windows, macOS and Linux. Contributions must therefore be easy
to review, safe for existing users, and explicit about their trade-offs.

## Ground rules

- Use English for pull requests, commit messages, review discussions, and new or
  updated code comments.
- Submit only changes that you have personally reviewed and understand. Disclose
  non-trivial AI assistance.
- Keep each pull request focused on one problem and exclude unrelated cleanup.
- Add relevant tests and report what you actually verified.
- Protect user privacy and preserve compatibility with existing data and devices.

## Before writing code

Small, self-contained fixes can go directly to a pull request. Open an issue or
discussion first for a large feature, a new dependency, or any change to privacy,
stored data, synchronization, authentication, or architecture.

Read the relevant [design documents](docs/design/) and [ADRs](docs/adr/) before
changing those areas. A large feature needs a design document in `docs/design/`,
with diagrams where possible (Mermaid renders on GitHub), so that it can be
maintained later. An architecture decision needs an ADR
([template](docs/adr/0000-template.md)); the maintainer will say when.

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<scope>): <subject>

<body>
```

`type` is one of `feat`, `fix`, `refactor`, `test`, `docs`, `perf`, `build`,
`ci`, `chore`. `scope` is the area the change is in, such as `sync`, `plugins`,
`ui` or `capture`. For example: `refactor(sync): stop publishing app_categories.json`.

The body is optional. It says what was wrong or missing before the change, such
as a bug, a feature users needed, or a slow step. The diff shows what changed.

## Language and comments

Write pull requests, commit messages and code comments in English. Translate the
comments of the code you change, and leave other Chinese comments alone.

Comments say only what the code cannot, such as a reason or a constraint. Put
design reasoning in an ADR or `docs/design/`, not in comments. Write Rust doc
comments as the
[rustdoc guide](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html)
describes.

## AI-assisted contributions

AI assistance is allowed, but you are responsible for the result. Put one of
these in the pull request description:

- `AI assistance: none`
- `AI assistance: used — <tools and affected files or areas>`

Before you ask for review, read the whole diff and be able to explain it. Check
that tests written by AI fail when the code is wrong. Pull requests that hide AI
use or show no self-review will be closed.

## Project-specific requirements

### Protect user privacy

- Never commit data from your own Hindsight, such as its database, screenshots,
  window titles or logs: it often names other people, and it stays in the git
  history.
- Never log window titles, URLs, OCR text, tokens, or OAuth responses at `info`
  or above. The default log filter prints `info` and up, so that detail belongs
  at `debug`.
- Treat any new network request, upload, telemetry, or data-retention behavior
  as a privacy change. Explain it explicitly before implementation and in the
  pull request.
- Treat changes to Tauri capabilities or the content security policy as security
  changes. Keep permissions narrow and justify each expansion.

### Preserve compatibility

- Never edit a database migration included in a release; add a new migration.
  A migration newly introduced by an unmerged pull request may still be revised
  during that pull request.
- One user's devices may run different Hindsight versions and sync with each
  other. In synced files, add new fields as optional, and never rename or remove
  existing ones.

### Respect every supported platform

Hindsight runs on Windows, macOS and Linux. Keep platform-specific code behind a
`cfg` check, and think about how the change behaves on each system. CI runs the
tests on macOS and Linux. On Windows it only checks that they compile, so test
Windows behavior on a Windows machine.

### Keep localization complete

Add each new string to all six files in `src/i18n/locales/`. A missing key
falls back to English without any error.

## Development and checks

Install the platform prerequisites from the
[Tauri documentation](https://v2.tauri.app/start/prerequisites/), Node.js 20 or
newer, and the Rust toolchain selected by `src-tauri/rust-toolchain.toml`.

```sh
npm ci
npm run tauri dev
```

Before submitting frontend or shared changes, run:

```sh
npm run format:check
npm run lint
npx tsc --noEmit
npm test
npx vite build
```

For Rust changes, build the frontend first because `tauri_build` expects `dist/`
to exist, then run:

```sh
npm ci
npx vite build
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Run the checks for what you changed. Markdown-only changes skip CI, so check how
they render. Passing CI does not replace trying the change yourself.

## Pull requests

Keep the pull request in draft until it is ready for review. Its English
description must include:

- what changed and why;
- a linked issue or ADR when one exists;
- commands run, results, and relevant manual verification;
- if the change depends on the platform (Windows, macOS or Linux), the platforms
  tested and not tested; otherwise, that the feature works;
- the impact on user data, sync and privacy, or "None";
- screenshots of a large UI change, taken in the web demo and saved in
  `docs/pr/<number>/`;
- the required AI-assistance declaration.

The template in `.github/pull_request_template.md` has a section for each.
Only the maintainer merges pull requests.

## License

Hindsight is licensed under the GNU General Public License, version 3 only
(`GPL-3.0-only`); see [LICENSE](LICENSE). Contributions are accepted under that
license unless a separate license is explicitly documented. Third-party code
and assets retain their respective licenses and notices.

Hindsight was previously distributed under MIT. Previously published MIT
versions retain their original permissions. The original notice below is
retained for portions received under MIT and included in this distribution;
it does not offer an MIT license for new GPL-licensed contributions.

<details>
<summary>Original MIT copyright and permission notice</summary>

```text
MIT License

Copyright (c) 2026 Tomotsugu

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

</details>
