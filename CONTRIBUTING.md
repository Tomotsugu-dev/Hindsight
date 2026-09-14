# Contributing to Hindsight

Thank you for helping improve Hindsight. Hindsight records sensitive activity
data and runs on both Windows and macOS. Contributions must therefore be easy to
review, safe for existing users, and explicit about their trade-offs.

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
changing those areas. If a decision constrains future work or rejects a
meaningful alternative, write an ADR using the
[ADR template](docs/adr/0000-template.md).

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<scope>): <subject>

<body: why the change is needed, not what it does — the diff shows that>
```

`type` is one of `feat`, `fix`, `refactor`, `test`, `docs`, `perf`, `build`,
`ci`, `chore`. `scope` is the module the change lives in: `capture`, `storage`,
`sync`, `repo`, `ai`, `chat`, `memory`, `ui`, `i18n`, `adr`. For example:
`refactor(sync): stop publishing app_categories.json`.

## Language and comments

Pull request titles and descriptions, commit messages, review discussions, code
comments, and Rust documentation comments must be written in English. New
standalone technical documentation should default to English; edits to an
existing document should preserve its language unless translation is the
purpose of the change. Localized UI text and language-specific user
documentation are exceptions.

This rule is prospective. When changing code, update and translate the comments
that document the changed code, even when a refactor preserves behavior. Leave
comments outside the changed area alone; translate them in a dedicated pull
request instead of burying functional changes in a large translation diff.

Write comments for the next reader:

- Explain intent, invariants, constraints, units, edge cases, and non-obvious
  trade-offs.
- Use `//!` for crate or module documentation, `///` for the function, type,
  field, or constant right below it, and `//` for a fact the code cannot say on
  its own: a constraint, an order that must not change, the local reason for one
  line. Design rationale belongs in an ADR or `docs/design/`; leave one sentence
  of conclusion in the code.
- Write every comment top-down. The first sentence says who uses the item and
  what it does; then what it changes; constraints last. A reader who stops after
  any sentence still has the most important part. For public interfaces,
  document relevant failure contracts under `# Errors`, `# Panics`, and
  `# Safety`.
- Put a nearby `// SAFETY:` comment on every `unsafe` block and `unsafe impl`.
  State the invariant that makes the operation sound.
- Give `TODO` comments an issue or ADR reference and a condition for removal.
- Update or remove comments made stale by the same change.

## AI-assisted contributions

AI assistance is allowed. Responsibility still belongs to the contributor.
Disclosure is required when AI generated or substantially rewrote a non-trivial
part of the submitted code, tests, documentation, or translation. Routine
completion of trivial syntax does not need line-by-line attribution.

Declare one of the following in the pull request description:

- `AI assistance: none`
- `AI assistance: used — <tools and affected files or areas>`

Put this declaration in the pull request description, not in source comments.
Before requesting review, inspect the final diff line by line, understand and be
able to explain it, verify APIs and dependencies against primary sources, check
licenses and edge cases, and demonstrate that generated tests fail when the
intended behavior is broken. Remove boilerplate, repetition, stale comments, and
unrelated changes.

Do not ask maintainers to debug output that you have not reviewed. Pull requests
that contain undisclosed AI-generated work or show no credible human self-review
will be closed without further review.

## Project-specific requirements

### Protect user privacy

- Never commit real activity history, screenshots, databases, window titles,
  OAuth tokens, API keys, machine identifiers, home paths, or other personal
  data. Use synthetic fixtures and redact diagnostic output.
- Never log window titles, URLs, OCR text, tokens, or OAuth responses at `info`
  or above. The default log filter prints `info` and up, so that detail belongs
  at `debug`.
- Treat any new network request, upload, telemetry, or data-retention behavior
  as a privacy change. Explain it explicitly before implementation and in the
  pull request.
- Treat changes to Tauri capabilities or the content security policy as security
  changes. Keep permissions narrow and justify each expansion.
- Preserve Hindsight's local-first behavior unless an accepted design decision
  says otherwise.

### Preserve compatibility

- Never edit a database migration included in a release; add a new migration.
  A migration newly introduced by an unmerged pull request may still be revised
  during that pull request.
- Assume devices running different Hindsight versions can synchronize with each
  other. Sync and storage changes must explain compatibility, migration, data
  loss, and rollback behavior.
- Add regression tests for bug fixes and tests for behavior changes whenever
  practical. Tests should verify externally meaningful behavior, not merely copy
  the implementation.

### Respect both supported platforms

Hindsight targets Windows and macOS. Keep platform-specific code behind the
appropriate `cfg` boundary and consider behavior on both systems. Test every
relevant platform available to you and state clearly which platform was not
tested. A green Windows CI run does not validate macOS-specific behavior.

### Keep localization complete

User-facing text belongs in the i18n resources, not inline in components. Keep
locale keys synchronized across the six files in `src/i18n/locales/`. If you
cannot verify a translation, call that out rather than presenting
machine-generated wording as reviewed.

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

Run the checks relevant to the final diff, plus any necessary platform-specific,
manual, ignored, or end-to-end tests. Markdown-only changes do not currently
trigger CI, so inspect their rendered output and links manually. Passing CI is
necessary, but it is not a substitute for reviewing behavior yourself.

## Pull requests

Keep the pull request in draft until it is ready for review. Its English
description must include:

- what changed and why;
- a linked issue or ADR when one exists;
- commands run, results, and relevant manual verification;
- tests added, or why a meaningful test is not practical;
- platforms tested and platforms not tested;
- privacy, security, migration, data loss, sync, and rollback impact where
  relevant;
- screenshots or a short recording for visible UI changes;
- the required AI-assistance declaration;
- confirmation that the final diff received a line-by-line self-review.

Resolve review threads and requested changes rather than marking them resolved
without addressing them. Maintainers may close incomplete or out-of-scope pull
requests, ask for a large change to be split, or close a pull request that has
not been self-reviewed. Only maintainers decide when a pull request is ready to
merge.
