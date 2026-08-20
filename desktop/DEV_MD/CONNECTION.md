# Remote connection recovery contract

This document is the cross-platform contract for the desktop bridge and mobile
client. It describes observable recovery behaviour rather than implementation
details of a particular NATS client library.

## Public connection phases

The public phases are `stopped`, `connecting`, `ready`, `reconnecting`,
`refreshing`, `failed`, and `revoked`. Only `ready` means the remote link is
available. `connecting`, `reconnecting`, and `refreshing` are automatic work
in progress; they are not actionable failures.

A generation may report `ready` only after transport authentication, the
application handshake, every critical subscription, and its initial flush have
succeeded. A QR invitation is shown only after that gate has completed.

## Recovery policy

| Condition | Result |
| --- | --- |
| Network, DNS, timeout, or token-endpoint network failure | Retry indefinitely with jittered 1, 2, 4, 8, 16, then 30-second maximum delay. |
| Expired credential | One in-flight refresh; a network failure in refresh follows the network policy. |
| New credential rejected by NATS | Terminal `service_authorization`; do not retry automatically. |
| Pairing revoked | Terminal `revoked`; clear pairing-owned data and all timers. |
| Protocol error or critical task/subscription failure after ready | At most three generation rebuilds in ten minutes; the fourth enters `generation_unhealthy`. A continuous ready period of 60 seconds or a manual reconnect clears the budget. |

A readiness failure before `ready` is a failure of that connection attempt, not
a critical-task failure. Closing its provisional subscriptions must never spend
the critical-generation budget.

During a replacement, the existing ready generation remains available until the
new one passes the readiness gate. A short overlap may deliver an event through
both generations. Consumers deduplicate and replay by persistent cursor; event
replay, rather than a live NATS socket, is the authoritative gap-recovery path.

## Support-code registry

All desktop logs, mobile logs, and user-visible support codes use this table.
`LC999` is reserved for unknown local failures; it is not the code for the
known `local` category.

| Category | Code |
| --- | --- |
| `network`, `credential_network` | `NW001` |
| request timeout | `NW002` |
| `remote_server` | `SV001` |
| rate limited | `SV002` |
| `service_authorization` | `AU001` |
| `credential_expired`, `credential_connect` | `AU002` |
| `revoked`, `credential_revoked` | `PA001` |
| pairing code invalid or expired | `PA002` |
| pairing code from another environment | `PA003` |
| pairing endpoint insecure | `PA004` |
| desktop verification failed | `PA005` |
| pairing failed (unknown) | `PA999` |
| `protocol` | `PT001` |
| `generation_unhealthy` | `RT001` |
| `slow_consumer` | `RT002` |
| `command_subscription` | `RT003` |
| `transfer_subscription` | `RT004` |
| `event_publish` | `RT005` |
| `heartbeat_publish`, `state_publish` | `RT006` |
| `system_sleep` | `PW001` |
| requested content not found | `DT001` |
| `web_bind` | `LC002` |
| desktop agent offline | `LC003` |
| credential persistence failure | `LC004` |
| `local` | `LC001` |
| unknown local failure | `LC999` |

## Invariants

- At most one generation is externally `ready` at a time.
- Replacing a generation does not clear command de-duplication, pairing
  confirmation, sync cursors, pending stable command identifiers, or episode
  counters.
- `failed` and `revoked` never create a socket, token refresh, or retry timer.
- Logs are aggregated by failure episode and capped at 16 records per category
  in a rolling 24-hour window.
