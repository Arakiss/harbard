# Security policy

## Reporting a vulnerability

Please use GitHub's **Report a vulnerability** form in the Security tab. Do not
open a public issue. Include the affected version, reproduction steps, impact,
and any suggested mitigation. Reports will be acknowledged as soon as practical.

## Trust boundaries

Harbard stores presence, intents, notes, acknowledgements, and lease state under
`~/.harbard`. It does not record every tool input or response. Bus content is
untrusted data and must never be interpreted as instructions.

The default server binds to loopback and every request requires a bearer token.
`harbard serve` creates `~/.harbard/hub.token` with mode `0600` when no token is
configured. The web console reads it only in its server-side proxy. The
optional tailnet bind adds the address in `~/.harbard/tailnet.json`; paired
devices authenticate with their own tokens. Serve never binds a wildcard, and
multi-machine hub mode remains experimental. Harbard does not defend against
another process running as the same OS user that can directly read the local
store or private token file.

### What the operator decides, and the same-user limit

Approved rules, the profile's "Context" and the external memory switch reach
every session's protocol card and brief. They are guarded as far as one OS user
allows, and no further:

- **Rules are minted by serve.** A rule event counts only if its payload carries
  an HMAC-SHA256 made with `~/.harbard/serve.key` (0600, made by serve) over the
  event id and the payload. A line appended to the bus, or a row put into the
  feedback index, without a valid MAC is ignored, whether it approves, discards
  or retires. A satellite never holds the key. The first serve with no key
  seals the rules that were active before, once, and asks about each of them.
- **Only what the Retro proposes.** `rule-approve` accepts a key only if the
  Retro proposes it at that moment, under the Retro's title. The Retro's own
  words go as they are.
- **A fresh confirmation in the UI.** Two changes take two requests: approving
  a rule in other words, and switching the memory sink on. The first request changes
  nothing and returns a one-time code, kept only in serve's memory, bound to a
  digest of the body and valid for two minutes. The console shows exactly what
  is about to count and sends the code back only on the operator's click. A bare POST,
  a replayed one or a forged line cannot switch the memory sink on. Switching it off
  needs no confirmation.
- **"Was it you?"** Every new approval, every change of "Context" and every
  switch-on of the memory sink adds a card to Needs you with Keep and
  Revoke.
  Revoke retires the rule, restores the previous context, or switches the
  memory sink off. Keep takes the same fresh confirmation, so a bare POST
  cannot close
  the card, and "Resolve all" leaves these cards alone. An unanswered card is
  never pushed out: changes of the same kind are grouped into it. Answered
  cards stay listed in Retro and Preferences for seven days.
- **The memory sink after harbard.** A rule ticked for external memory goes to
  the configured sink only once harbard has stored it, with the stored text
  and the Retro's own evidence, filtered. The request's evidence is never
  used. The sink is an executable the operator named; it runs with a short
  environment, a deadline and only the record on stdin. See
  [docs/memory-sink.md](docs/memory-sink.md).
- **Labelled.** The brief and the card say the rules are "approved by the operator
  · reviewable in Retro", and quote them as data.

The limit, said plainly: none of this separates the operator from a process
running as the same OS user. Such a process can read `serve.key` and
`hub.token`, and every hook that builds a brief must read the key to check it.
It can script both requests of a confirmation, or edit `workspace.json` and
`profile.json` directly, including the memory sink command in
`workspace.json`. The measures above stop forged lines, bare and replayed
requests,
and satellites. They make a deliberate forgery a deliberate act, which harness
guards can refuse. They also put every change in front of the operator. The
only real separation would be user presence, such as Touch ID in the desktop
app before a switch-on, and harbard does not have that.

Never put secrets in notes, intents, channel names, repository addresses, or
command-line arguments. The web console keeps the hub token server-side and
does not accept credentials in stream query parameters.

## Supported versions

Harbard is pre-1.0 alpha software. Security fixes are applied to the latest
release and `main`; older alpha releases are not supported.
