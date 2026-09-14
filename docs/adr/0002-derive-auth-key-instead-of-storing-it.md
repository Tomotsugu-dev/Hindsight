# ADR-0002 · Derive the `refresh_token` encryption key instead of storing it

- **Date**: 2026-09-12 (implemented on 2026-05-09; recorded retrospectively)
- **Status**: **Accepted**
- **Related**: commit `aae2748` · commit `ed33a1e` · `sync::auth::derive_master_key`

## Context

Google sign-in returns a `refresh_token`. Hindsight must retain it locally, or
the user must repeat browser authorization every time the app starts. Storing it
in plaintext is unacceptable, so it must be encrypted—which raises a second
question: **where should the encryption key live?**

Keeping the key beside the ciphertext provides no protection when the database
is copied. Asking the user for it on every launch would prevent unattended
background sync. The conventional answer is the operating system's credential
store.

We tried that. On both platforms, the key could become inaccessible without any
action by the user. Once it was gone, the ciphertext in the database could never
be decrypted again. Restarting or reinstalling Hindsight did not help; the user
had to repeat OAuth.

On Windows, Credential Manager entries are protected by DPAPI, whose master key
is derived from the user's login password. When the user changes the password,
Windows can decrypt and rewrap the master key. When an administrator resets it
without the old password, Windows cannot, and the old entries become
inaccessible. Microsoft's documentation states that local accounts have no
domain backup key; the only recovery is to restore the previous password. Users
have also reported entire sets of entries disappearing after profile migration,
`VaultSvc` failures, or some cumulative updates, although these cases are not
documented well enough to establish a cause.

On macOS, Keychain access control identifies the writing application by its
Designated Requirement. An ad-hoc signature has no stable requirement because it
is tied to the binary hash. Changing one line of code changes the identity, so a
new build cannot read the key written by the previous build.

The repository records two attempts to address this. Commit `aae2748`
(2026-05-04) treated key access as a transient failure and stopped overwriting
the old key after a failed read. Five days later, commit `ed33a1e` (2026-05-09,
v0.4.5-beta) removed `keyring = "3"` entirely.

**If we make no decision, users can be signed out without taking any action, and
the failure cannot recover by itself.**

The macOS constraint had already disappeared one day before keyring was removed:
commit `8944973` introduced Developer ID signing and notarization, giving the app
a stable Designated Requirement. If we reconsider the credential store today,
Windows DPAPI is the remaining blocker.

## Decision

**Do not store the key.** Derive it whenever it is needed from two stable,
machine-local values that are not stored in the database:

```
SHA256(purpose_string ‖ machine_id ‖ user_home)
```

`machine_id` is the platform's stable machine identifier: the Windows registry
`MachineGuid`, macOS `IOPlatformUUID`, or Linux `/etc/machine-id`. `user_home` is
the user's home-directory path. The current purpose string is
`hindsight-auth-v1`.

## Alternatives

Derivation is the only remaining option that neither stores plaintext in the
database nor creates another stored secret that can disappear.

| Option | Benefits | Costs | Why not chosen |
|---|---|---|---|
| **Derive on demand (chosen)** | Nothing can be lost; a copied database is useless off the original machine | Does not protect against an attacker logged in as the user | — |
| Operating-system credential store | System-level protection | Inaccessible keys make the ciphertext permanently useless | **Tried and failed** (see Context) |
| Store plaintext in the database | Simplest implementation | A copied database exposes the token | Provides no protection |
| Store the key in a separate file | Simple | The key is copied with the ciphertext | Equivalent to no encryption |
| Ask the user for a password | Strongest protection against a copied database | Prevents unattended background sync | Conflicts with automatic sync |

## Consequences

**Moving to another machine, changing the home-directory path, or reinstalling
the operating system requires signing in again.** This is the other side of the
same property: if another machine cannot derive the key, neither can the user's
replacement machine.

**This encryption does not protect a compromised local account.** An attacker
logged in as the user can read the machine ID and home path and derive the key.
It protects only against the database file being copied in isolation. Once the
local account is compromised, the attacker can already access browser cookies
and saved passwords; this layer cannot and is not intended to defend against
that threat.

**The stability of derivation inputs is an external dependency.** If a platform
changes the behavior of its machine identifier, users on that platform will
have to sign in again.

**Every purpose requires a distinct purpose string.** The seed is fixed as the
machine ID plus home path. A separate purpose string is what keeps future keys,
such as the deletion-tag key proposed by ADR-0001, independent. Changing the
value of an existing purpose string signs out every user, so it must remain
stable.

## Impact on user data

- **Existing data**: Ciphertext from the credential-store era cannot be opened
  with the derived key. When `refresh_and_persist` detects decryption failure, it
  clears `auth_state` and returns the UI to the signed-out state. The user signs
  in once, after which the key remains stable. This transition occurred in
  v0.4.5-beta.
- **Migration**: None. The `auth_state` schema does not change.
- **Rollback**: A version earlier than v0.4.4 reads its key from the credential
  store and cannot open ciphertext produced with the derived key. It likewise
  clears the state and requires one sign-in. No user data is lost.
- **Irreversible effects**: None.

## Follow-up

Reconsider this decision when:

- **Returning to the operating-system credential store**: macOS is no longer a
  blocker; a viable design must first address Windows DPAPI password resets.
- **Defending against a compromised local account**: a user-provided secret must
  participate in derivation, at the cost of unattended background sync.
- **A platform's machine identifier becomes unstable**: replace that platform's
  derivation input, signing its users out once during the transition.
- **A second derived key is needed**, such as ADR-0001's deletion tag: reuse the
  seed with a different purpose string; do not reuse `AUTH_KEY_CONTEXT`.

## References

Windows: [DPAPI MasterKey backup failures](https://learn.microsoft.com/en-us/troubleshoot/windows-server/certificates-and-public-key-infrastructure-pki/dpapi-masterkey-backup-failures) · [Windows Data Protection](https://learn.microsoft.com/en-us/previous-versions/ms995355\(v=msdn.10\))

macOS: [Technical Note TN2206: macOS Code Signing In Depth](https://developer.apple.com/library/archive/technotes/tn2206/_index.html) · [Apple Developer Forums: consequences of changing a signing certificate](https://developer.apple.com/forums/thread/669350)
