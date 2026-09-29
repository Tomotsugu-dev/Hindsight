# ADR-0000 · <State the decision>

- **Date**: YYYY-MM-DD
- **Status**: Proposed / **Accepted** / Deprecated / Superseded by ADR-NNNN
- **Related**: ADR-NNNN · issue #N · PR #N · commit `abc1234`

<!--
Start with one or two sentences: the problem and what we decided. Then the
flow. The sections after it hold only what a diagram cannot show: why this
option, what was rejected, and the effect on existing data and older versions.
-->

<One or two sentences: the problem, and what we decided.>

## Flow

<!--
Required when several devices or components interact, or when the order of
steps matters. Use Mermaid; GitHub renders it.
- Sequence diagrams: the main path first, then each case that depends on
  timing (a crash, two devices acting at the same moment).
- A state diagram when something moves between states.
- A decision table: every case and what happens. Each row becomes a test.
Otherwise write "No flow." and keep the ADR to text.
-->

## Context

<!-- The problem, its impact, and the constraints, as short bullets. -->

## Decision

We will **X**.

<!-- Only what the flow does not show: invariants, exceptions, boundaries. -->

## Alternatives

<!-- Viable options only, one or two lines each: the deciding reason against it. -->

- **X (chosen)**: ...
- **Y**: ...

## Consequences

<!-- State the main trade-off first. -->

- **Benefits**: ...
- **Costs and risks**: ...

## Data, compatibility, security, and privacy

<!--
Required for changes to stored data, configuration, sync, network behavior,
authentication, permissions, or sensitive data. Otherwise write "No impact."
-->

- **Existing data and migration**: ...
- **Mixed versions and rollback**: ...
- **Irreversible effects**: ...
- **Security and privacy**: ...

## Verification

<!-- The tests (one per decision-table row) and what to check on a real device. -->

## Follow-up

<!-- Optional: next actions or concrete conditions for revisiting the decision. -->

- ...

<!--
Copy to `docs/adr/NNNN-short-title.md`. Keep it short: a busy reader should
finish it.
To change an accepted decision, create a new ADR that supersedes the old one.
-->
