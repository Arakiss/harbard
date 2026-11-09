<!-- Please use a Conventional Commit title: `<type>(<scope>): <subject>`. -->

## Summary

<!-- One or two sentences on what this PR does and why. -->

## Motivation

<!-- What problem does this solve? Link the issue or discussion if one exists. -->

## Invariant impact

<!-- Tick what applies. The invariants are the constitution in SPEC.md. -->

- [ ] No impact (docs or tests that do not change behavior).
- [ ] **Hooks.** Every installed hook stays timeout-bounded and fail-open, and none can deny a tool call.
- [ ] **Bus content.** Stored text is still rendered as quoted, inert data.
- [ ] **Serve.** Every request still requires a token, and the bind stays loopback plus the configured tailnet address.
- [ ] **Store.** Event segments stay append-only.

## Surface changes

<!-- Leave blank if none apply. -->

- [ ] Changes a CLI verb, flag, or `--json` output.
- [ ] Changes the protocol catalog (`harbard/protocol/v1`).
- [ ] Changes an HTTP route.
- [ ] Changes the on-disk layout under `~/.harbard`.

## Testing

```sh
just ci
```

## Notes for reviewers

<!-- Pitfalls, follow-ups, known limitations, alternatives considered and rejected. -->
