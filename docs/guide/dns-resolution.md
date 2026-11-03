# Hostname resolution in this process (static musl builds)

`future` resolves host names itself when the system's own configuration is not
where its C library looks. This page explains when that happens, which files are
read, the two settings that override them, and what is measured versus assumed.

Scope: every HTTP client built through `future-http` — the platform API, skill
and MCP servers, downloads, and the Agent's and host's own requests — plus the
CLI's TCP Agent endpoint (gRPC). **Not** covered: the NATS relay (`async-nats`
resolves inside itself — `ToServerAddrs::socket_addrs` →
`tokio::net::lookup_host` — and offers no hook to replace that), and any lookup
some other code makes through `getaddrinfo` directly.

## 1. What a static musl build cannot do

The Linux CLI release is linked against musl statically. Three consequences
cannot be configured away:

| Limitation | What it means |
|---|---|
| No NSS | musl has no `nsswitch.conf` and no loadable name-service modules. mDNS/Avahi names, LDAP directory names, and systemd-resolved's per-link or split-DNS views are simply not available. |
| Hard-coded paths | musl's resolver opens `/etc/hosts` (`src/network/lookup_name.c`) and `/etc/resolv.conf` (`src/network/resolvconf.c`). There is no `RESOLV_CONF`-style environment variable, and no `$PREFIX`-relative fallback. |
| No `dlopen` | A static binary cannot load a plugin later, so "install an NSS module" is not a fix for a static release — that is why the resolver below exists instead. |

The first two are why a machine can have working DNS and still fail every lookup
from `future`: the nameserver list may be in a file musl never opens.

Measured on a musl-linked binary in a `chroot` with no `/etc` at all
(`aarch64`, musl 1.2.x):

| `/etc/hosts` | `/etc/resolv.conf` | `getaddrinfo("example.com")` | `getaddrinfo("localhost")` |
|---|---|---|---|
| absent | absent | fails after **5.0 s** — `EAI_AGAIN`, "Try again" | fails after **5.0 s** — `EAI_AGAIN` |
| present | absent | fails after **5.0 s** — `EAI_AGAIN` | resolves from the hosts file (µs) |
| absent | present | resolves (defaults to the file's nameserver) | fails immediately — `EAI_NONAME`, "Name does not resolve" |
| present | present | resolves | resolves (µs) |

Two different failures, and they are worth telling apart: a missing
`resolv.conf` stalls every DNS lookup for five seconds before `EAI_AGAIN`;
a missing `hosts` file only breaks names that live in it (`localhost` above all)
and fails instantly. The five-second stall is musl's own timeout, not a network
problem, which is why it looks like "the network is down" in a bug report.

## 2. When the process resolves on its own

One decision, made once per process, from the environment and the filesystem:

| Situation | Plan | Behaviour |
|---|---|---|
| `FUTURE_DNS_SERVER` set and non-empty | custom | that address is the **only** nameserver; no `resolv.conf` is read |
| `FUTURE_RESOLV_CONF` set and non-empty | custom | that file is read instead of the search path |
| `/etc/resolv.conf` exists (and neither variable is set) | **system** | nothing is installed: every lookup is `getaddrinfo`, exactly as before |
| no `/etc/resolv.conf`, but a candidate below exists | custom | the first existing candidate is read |
| nothing exists | custom | lookups fail with a message naming what was checked |

The third row is the contract: on an ordinary Linux (and on macOS, where the
same file exists), this feature is inert. See §6 for how that is pinned.

**Windows** is the exception that keeps the contract honest: its `getaddrinfo`
is the Windows DNS client, not a reader of `/etc/resolv.conf`, so a missing file
there is no evidence that the process cannot resolve. Without an explicit
setting the plan stays **system** on Windows; an explicit `FUTURE_DNS_SERVER` /
`FUTURE_RESOLV_CONF` is still honoured.

Who attaches the resolver matters as much as the decision: it is attached in
`future-http`'s client builders, so the Agent, the host, the CLI and the TUI all
follow it — and the CLI's TCP Agent endpoint (gRPC) reuses it through its own
connector. The NATS relay is **not** covered (see the top of this page).

## 3. Candidate paths

`resolv.conf`, first existing wins:

1. `/etc/resolv.conf`
2. `/run/systemd/resolve/resolv.conf` — systemd-resolved's own file, normally
   what `/etc/resolv.conf` symlinks to
3. `/run/resolvconf/resolv.conf` — openresolv
4. `$PREFIX/etc/resolv.conf` — Termux, when `PREFIX` is set

Hosts files, all existing ones are consulted:

1. `/etc/hosts`
2. `$PREFIX/etc/hosts` — Termux

A missing hosts file is never an error: that is also how libc behaves. Only the
first existing `resolv.conf` is used; two files are never merged, because a
machine with two configured views of the same namespaces is a machine whose
administrator has already decided which one applies.

## 4. The two settings

| Variable | Value | Effect |
|---|---|---|
| `FUTURE_DNS_SERVER` | `1.1.1.1`, `1.1.1.1:5353`, `::1`, `[::1]:53` | Sole nameserver. No `resolv.conf` is read, so this works on a machine with no `/etc` at all. The hosts files above are still consulted first, as they are by libc. A value that is not an IP address is reported as a configuration error rather than ignored. |
| `FUTURE_RESOLV_CONF` | a path | Read that file instead of the search path, even when `/etc/resolv.conf` exists. A path that does not exist is reported as a configuration error — silently falling back to the system would hide the mistake. |

`options` and the search list are honored from whichever file is read
(`search`/`domain`, `ndots`, `timeout`, `attempts`, with the same clamping libc
applies). `sortlist`, `rotate` and EDNS knobs are ignored: this resolver sends
one A and one AAAA query per name and has nothing to rotate.

## 5. What the resolver implements, and what it does not

Implemented: A and AAAA over UDP, transaction-id checking, NXDOMAIN, `rcode`,
name-compression skipping, search-list and `ndots` handling, per-attempt
timeout and retry count, hosts-file lookup with aliases and case-insensitive
matching, and deduplicated answers.

Deliberately not implemented:

- **No TCP fallback.** A truncated reply is reported instead of retried over
  TCP. A single A/AAAA answer cannot fill 512 bytes, so this is a theoretical
  hole, not a practical one.
- **No caching.** A lookup costs a query, the same as `getaddrinfo` did.
- **No DNSSEC, no EDNS, no DoH/DoT.** Nothing to configure; if you need them,
  put them in the resolver the nameserver address points at.
- **No NSS.** See §1 — mDNS, LDAP and per-link views remain unavailable.

## 6. How "the default is unchanged" is pinned

`Plan::detect` is a pure function over the environment and a file-existence
probe, so the decision is tested without touching the real `/etc` or racing
other tests through `std::env::set_var`. Two tests guard the contract:

- `dns::config::tests::an_ordinary_linux_keeps_the_system_resolver` asserts
  that a machine with `/etc/resolv.conf` produces `Plan::System`.
- `dns::tests::the_system_plan_attaches_no_resolver` asserts that a system plan
  attaches no `reqwest` resolver and no gRPC resolver, and that a custom plan
  attaches both.

Both were falsified once before landing: short-circuiting the `/etc/resolv.conf`
case (making every machine "custom") fails the first; returning `None` from the
custom arm fails the second. On the gRPC side,
`future_rpc::transport::tests::a_supplied_resolver_decides_where_a_tcp_endpoint_connects`
asserts both directions — a supplied resolver is consulted for a name that does
not exist anywhere, and the same endpoint still fails through `getaddrinfo`
without one.

## 7. Environment checklist

| Environment | What it needs |
|---|---|
| Ordinary Linux / macOS | nothing. Do not set either variable. |
| Container with no `/etc` | `FUTURE_DNS_SERVER`, or mount a `resolv.conf` and name it with `FUTURE_RESOLV_CONF`. Bind-mounting the host's `/etc/resolv.conf` is the smallest fix when you control the container. |
| Minimal rootfs / initramfs | Same as above. If the machine's DNS is managed elsewhere (a local stub resolver on `127.0.0.1:53`), point `FUTURE_DNS_SERVER` at it. |
| Termux | `$PREFIX/etc/resolv.conf` is picked up automatically when `/etc/resolv.conf` does not exist. `localhost` still needs `$PREFIX/etc/hosts`; if Android provides neither, set `FUTURE_DNS_SERVER`. |
| Corporate split DNS / mDNS / LDAP names | **not solved.** These need NSS, which a static musl build cannot load. Use a dynamically linked build, or have the names resolved by a stub resolver reachable as a plain nameserver. |

## 8. What a failure looks like

```
cannot resolve "api.example.com": no resolv.conf is readable, so this process
has no nameserver. Resolution in this process: no resolv.conf found; checked
/etc/resolv.conf, /run/systemd/resolve/resolv.conf,
/run/resolvconf/resolv.conf; hosts: /etc/hosts (missing). Set
FUTURE_DNS_SERVER=1.1.1.1 (or 1.1.1.1:53), set
FUTURE_RESOLV_CONF=/path/to/resolv.conf, add the name to one of the hosts files
above, or use an IP address instead of a host name. A static musl build reads
only these files and cannot use NSS modules (mDNS, LDAP, systemd-resolved).
```

Request-level failures carry the same text: the CLI renders a `reqwest` error
with its cause chain (`crate::dns::describe`), because `reqwest::Error`'s own
`Display` prints only "error sending request for url (…)" and hides the
resolver's explanation one level down.

The gRPC path resolves a TCP endpoint once **before** connecting
(`dns::preflight`), for the same reason: `tonic` renders every connector failure
as `transport error`, so a missing nameserver and a refused port used to be
indistinguishable. The connector still performs its own resolution when it
connects; the pre-flight only decides whether the failure is worth explaining.
