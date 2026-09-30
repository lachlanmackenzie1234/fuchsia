<!-- Copyright 2026 The Fuchsia Authors. All rights reserved.
Use of this source code is governed by a BSD-style license that can be
found in the LICENSE file. -->

# driver-lab proxy driver specification

This is the normative specification for the engineering-only driver-lab
proxy driver (`lab_proxy`) at `//src/devices/driver-lab`. The companion
host tooling specification lives at `//tools/driver-lab/SPEC.md`.

The specification is delivered in two phases. Phase 1 -- unclaimed-node
binding through existing Driver Framework mechanisms, with no Driver Manager
changes -- is delivered first. Its contracts are designed takeover-ready so
that phase 2 -- managed takeover, in which Driver Manager temporarily
replaces a node's normal driver with the proxy and later restores it --
requires no major version change.

## Phase 1: unclaimed-node proxy

### Implementation status

Changeset numbers (CS) are global across this specification and
`//tools/driver-lab/SPEC.md`, whose changesets interleave with these;
gaps in the numbering below are driver-lab changesets. Every changeset
marks itself complete in this list when it lands, so at any commit this
list shows which changes precede and which follow that commit.

- [x] CS1 `[driver-lab] Proxy driver specification` -- this
      document.
- [ ] CS3 `[driver-lab] Phase 1 proxy driver core` -- the
      takeover-ready `fuchsia.driver.lab` wire contract (sections 7.4,
      11) and the host-testable policy core: ceiling and exact
      allowlist enforcement (9, 10), per-boot identity and description
      digests (8.3, 10.2), bounded reads and snapshots (12.1, 12.5),
      session lifecycle and leases (16), and the audit ring (18). Bind
      rules are `false` and no resources are offered.
- [ ] CS9 `[driver-lab] Platform resource provider, bind rules,
      realm tests` -- resource acquisition and local MMIO mapping
      (8.4, 15), the `PROXY_TARGET` bind library and property-gated
      binding (7.2, 7.3), driver realm tests (23.3), and the lab_root
      test fixture serving a fake platform device (23.3, and the
      live-activation path documented by CS10).

Remaining phase 1 work, not yet scheduled as changesets:

- [ ] Generated target policy: authored, reviewed ceiling manifests
      replacing the engineering default ceiling (9.2, 9.6, 22).
- [ ] Engineering assembly inclusion and production-absence
      verification (7.1, 25).
- [ ] Bounded sequences and masked mutation: `Write32`, `Poll32`,
      delays, barriers, `ExecuteSequence`, and the mutating session
      path (11.4, 11.5, 12.2-12.4, 13; milestone P3).
- [ ] Protocol-resource adapters (8.1, 9.5, 11.6; milestone P4).
- [ ] Interrupt observation (11.7, 17; milestone P5).
- [ ] Stop-path hardening -- cancellation of in-flight work, epitaphs,
      bounded deadlines (19) -- per-class rate and count limits (12.1),
      and structured configuration (22).

Phase: 1 -- new-driver development. The proxy binds to unclaimed nodes through
existing Driver Framework mechanisms. Managed takeover is specified in the
Phase 2 section of this document; the wire contract here reserves the fields
it requires.

Scope: standalone proxy driver, target policy, resource execution, audit, and
lifecycle.

Companion specification: host tooling at `//tools/driver-lab/SPEC.md`

### 1. Purpose

The proxy driver provides exclusive, resource-scoped hardware access on a
Fuchsia target for interactive and agent-assisted driver development. It runs
locally on the target, maps only resources offered to its device node, executes
bounded operations, enforces a target ceiling and session allowlist, and
records a target-side audit trail.

The proxy will ultimately support two deployment situations:

1. it is selected as the driver for an unclaimed device node during
   new-driver exploration (phase 1, specified here); or
2. Driver Manager temporarily replaces the node's normal driver with the
   proxy during managed takeover (phase 2).

The same proxy implementation serves both situations; this section specifies
the first and keeps the contract takeover-ready. Embedding the proxy as a
library in a production driver is not part of the baseline design.

### 2. Normative language

The terms **must**, **must not**, **should**, and **may** describe requirements,
recommendations, and optional behavior respectively.

The following terms have specific meanings:

- **Proxy:** The standalone DFv2 driver defined here.
- **Target ceiling:** Immutable target policy describing the greatest access
  the proxy may provide for a node/resource class.
- **Session allowlist:** Exact plan-derived permissions supplied when a session
  opens. It may only narrow the target ceiling.
- **Managed takeover:** Driver Manager stops a node's current driver, binds the
  proxy, and later restores the original driver. Defined in the Phase 2
  section of this document.
- **Resource identity:** A stable logical description and digest for an offered
  resource. It never includes a host-provided physical address.
- **Partial completion:** An execution in which at least one operation began
  but the complete plan did not finish successfully.

### 3. Goals

The proxy must:

1. Run in a dedicated driver host.
2. Obtain resources only from the node to which it is bound.
3. Support platform-, PCI-, and other parent-native resource providers through
   narrow backend adapters.
4. Map MMIO locally with device-appropriate cache policy.
5. Retain all objects required for each mapping's lifetime.
6. Represent resources by stable logical IDs and names.
7. Describe resource size, kind, supported operations, target ceiling, and
   identity digest.
8. Enforce both the target ceiling and the exact session allowlist immediately
   before every access.
9. Execute bounded MMIO reads, snapshots, masked writes, polls, delays, and
   barriers.
10. Support resource-specific GPIO, SPI, I2C, serial, clock, reset, and
    interrupt operations without erasing their production semantics.
11. Execute timing-sensitive sequences locally when explicitly requested.
12. Serialize mutation and reject ambiguous concurrent execution.
13. Report explicit per-operation status and partial completion.
14. Record attempted, rejected, successful, failed, and cleanup operations in a
    bounded audit ring.
15. Close or cancel every outstanding request during stop.
16. Contain an experimental driver fault away from Driver Manager and unrelated
    production drivers.
17. Be included only in authorized engineering configurations and remain
    inactive until explicitly bound.

### 4. Non-goals

The proxy must not:

- accept an arbitrary physical address;
- request root resource;
- export MMIO VMOs, interrupt handles, BTIs, IOMMUs, or root-like capabilities
  to the host;
- map resources belonging to another node;
- share ownership with the normal driver for the same node;
- inspect the memory or private state of an active production driver;
- run inside Driver Manager;
- execute arbitrary uploaded code or target-side scripts;
- load a host-provided policy that widens the target ceiling;
- claim transactional rollback for MMIO;
- automatically restore every register from captured "before" values;
- permit unbounded polling, sleeping, snapshots, vectors, or audit output;
- expose DMA programming, arbitrary PCI configuration writes, or SMC calls in
  the initial protocol;
- bind broadly to normal nodes without explicit Driver Manager selection or
  engineering-only bind metadata;
- coordinate operations across multiple device nodes in one session;
- guarantee sub-millisecond timing precision for delays, polls, or
  inter-operation gaps in V1; or
- appear in a production image.

### 5. Architecture

```text
Host tooling
    |
    | ffx target transport
    v
Driver Manager control plane
  discovery: fuchsia.driver.development
  takeover (phase 2): engineering-only protocol
    |
    | bind fixed proxy / route channel
    v
Resource-scoped proxy in isolated driver host
    |
    +-- policy and session server
    +-- MMIO executor
    +-- protocol-resource adapters
    +-- interrupt tracker
    +-- audit ring
    |
    v
Resources offered by the selected device node
```

Driver Manager owns discovery and, in phase 2, takeover and restoration. The
proxy owns only the experimental session and hardware execution after it is
bound.

### 6. Design invariants

#### 6.1 Capability confinement

The proxy can access only resources offered to its bound node. It must use the
parent's normal platform, PCI, or bus protocol. It must not obtain a global
resource and reconstruct mappings from host input.

#### 6.2 Exclusive ownership

The normal driver and proxy must never independently own or operate the same
hardware block at the same time. In phase 1 the proxy binds only to nodes with
no bound driver; during managed takeover (phase 2), the normal driver is fully
stopped and unbound before the proxy obtains resources.

Bind order is not an ownership mechanism.

#### 6.3 Target ceiling is authoritative

The host may provide an exact read consent list and an approved plan, but it
cannot:

- add a resource;
- increase a resource size;
- add an operation width;
- remove a hard deny;
- increase a target deadline;
- enable a disabled resource class; or
- widen a write mask.

#### 6.4 Local execution

MMIO mappings and interrupt objects remain on the target. FIDL carries scalar
operations, protocol transactions, results, descriptions, and audit entries.

#### 6.5 Programming-model fidelity

The target wire contract should preserve the meaning of normal driver
operations:

- protocol-backed resources retain production FIDL request and response
  semantics where possible;
- MMIO remains explicitly width- and offset-based;
- interrupt waiting remains asynchronous;
- reset and clock state changes remain explicit;
- SPI/I2C/serial exchanges do not become generic byte writes; and
- errors are not collapsed into prose.

A generic sequence protocol may coordinate heterogeneous operations locally,
but it must identify the original resource operation precisely. It is an
execution mechanism, not a replacement programming model.

#### 6.6 Bounded execution

Every request has target-enforced bounds. No host field can select an infinite
timeout, unconstrained vector, or blocking dispatcher sleep.

#### 6.7 Explicit partial completion

Writes generally cannot be undone. Execution reports every operation that
began, stops according to declared error rules, and never labels the sequence
atomic.

#### 6.8 Audit is diagnostic, not durable authorization

The proxy audit establishes what it attempted under a run/session context. It
does not prove that the host approval was issued by a particular human.

### 7. Deployment and activation

#### 7.1 Engineering-only packaging

The proxy package should be present by default in the engineering
configurations that support driver-lab workflows. Presence alone grants no
hardware access.

Production images must not offer any of the following, and the build must
verify this:

- the proxy driver package;
- the Driver Manager takeover protocol (phase 2);
- special debug bind metadata;
- target debug policy artifacts; or
- debug capability routes.

The verification mechanism (package absence, a Driver Manager binary variant,
or config gating with capability-route verification) is owned by platform
assembly. This specification requires only that the chosen mechanism be
verifiable at build time.

#### 7.2 Inactive by default

The proxy must not opportunistically claim normal device nodes. In phase 1 it
becomes eligible only through explicit engineering-only node metadata or a
developer-created test node; it may be loaded through engineering assembly
inclusion or ephemeral registration (`fuchsia.driver.registrar`). Plans
cannot supply a driver URL.

In phase 2 a Driver Manager operation may additionally select the fixed proxy
identity for an exclusive managed takeover. That node-scoped force-bind is
new Driver Manager work (the existing `DisableDriver` is global by URL and
unsuitable) and is specified in the Phase 2 section of this document.

#### 7.3 Direct selection during development

For hardware without a normal driver, an engineering product or explicit
developer action selects the proxy at initial bind. This is the phase 1
activation path. It uses the same policy, FIDL, evidence, and session
implementation that managed takeover (phase 2) will reuse.

#### 7.4 Managed takeover (phase 2)

Managed takeover -- stale-expectation checks, the exclusive takeover lease,
orderly stop/unbind of the original driver, restoration, and how the proxy
receives run/takeover identity -- is specified in the Phase 2 section of this
document. The phase 1 wire contract reserves the takeover-identity fields so
that flow requires no major version change.

### 8. Resource model

#### 8.1 Resource kinds

The extensible resource model begins with:

- MMIO region;
- interrupt;
- GPIO protocol;
- SPI protocol;
- I2C protocol;
- serial protocol;
- clock protocol; and
- reset protocol.

Additional kinds require explicit capability descriptions, bounds, tests, and
threat-model review.

#### 8.2 Resource description

Each resource description contains:

- numeric ID scoped to the proxy instance;
- stable logical name when available;
- resource kind;
- provider identity/type;
- logical size where applicable;
- supported access widths or protocol methods;
- read/write classification;
- target limits;
- stable metadata used to compute a resource-description digest; and
- flags identifying missing or provisional metadata.

Descriptions never expose physical addresses or raw target handles.

#### 8.3 Resource digest

The digest is computed from canonical descriptions including:

- node identity criteria;
- provider kind;
- resource ordinal/name;
- logical size;
- operation widths;
- the immutable target-ceiling entries that apply to this resource, not the
  whole policy manifest; and
- relevant stable parent metadata.

Dynamic handle values, mapping addresses, boot IDs, and service-instance names
are excluded.

The digest allows a persistent host read grant to stop matching when resource
layout or policy changes. Scoping the ceiling input per resource ensures that
a policy change on one resource does not invalidate grants on unrelated
resources.

#### 8.4 Parent adapters

Use an injected `ResourceProvider` interface with implementations appropriate
to the target node:

```text
ResourceProvider
    DescribeResources()
    OpenMmio(id)
    OpenInterrupt(id)
    ConnectProtocol(id, kind)

PlatformResourceProvider
PciResourceProvider
Other reviewed bus-specific providers
FakeResourceProvider
```

Do not place bus-specific address translation in the FIDL server or policy
engine.

If a parent exposes a larger region than the proxy's logical resource, checked
translation must verify both the logical resource and actual backing mapping:

```text
backing_offset = logical_base + request_offset
```

All arithmetic is checked before access.

Resource providers communicate with the parent over zircon channel-transport
FIDL. The isolated driver-host requirement (goal 1) already forces this:
driver-transport (`fdf`) protocols require colocation with the parent.
Parents that offer resources only over driver transport are ineligible for
proxy access in V1; whether a colocated mode is ever introduced is a phase 2
open decision.

### 9. Target policy

#### 9.1 Policy layers

The proxy evaluates:

```text
actual resource bounds
    ∩ immutable target ceiling
    ∩ session allowlist
    ∩ session mode
    ∩ per-operation structural validation
```

Every layer must permit the operation.

#### 9.2 Immutable target ceiling

The ceiling comes from engineering assembly, board/bus metadata, or immutable
package content. Runtime configuration may narrow it but cannot widen it.

It specifies:

- permitted resource providers and resource kinds;
- maximum logical region sizes;
- access widths;
- hard-denied read offsets or ranges;
- whether unknown reads may be enabled by a session grant;
- exact writable offsets and masks;
- precondition/readback requirements;
- permitted protocol methods and maximum transaction sizes;
- interrupt observation permission;
- maximum operations, snapshot items, poll time, delay, and sequence duration;
- audit capacity; and
- cleanup rules.

#### 9.3 Unknown reads

For a resource whose ceiling permits operator-authorized exploration, an
otherwise unclassified aligned read may be allowed only when:

- the offset fits the logical and actual resource;
- it is not hard-denied;
- the exact resource, offset, width, and access class appear in the session
  allowlist; and
- all rate, count, and duration limits are satisfied.

This is an engineering risk decision. The proxy does not claim that an allowed
unknown read is side-effect free.

A session request to sweep a range is expanded into exact offsets before the
session opens. The proxy does not accept a wildcard "all offsets in this
region" allowlist rule.

#### 9.4 Writes

Writes are denied unless the immutable target ceiling contains an exact
writable-register entry. A host/session grant cannot invent a write entry.

Each entry specifies:

- offset and width;
- allowed mask;
- whether read-modify-write is valid;
- allowed precondition mask;
- whether a precondition is required;
- readback mask and semantics;
- barrier requirements;
- failure classification; and
- optional explicitly reviewed cleanup.

Write-one-to-clear, self-clearing, FIFO-like, or destructive-read registers
require a distinct operation semantic. Do not overload ordinary masked writes.

#### 9.5 Protocol-backed resources

For GPIO, SPI, I2C, serial, clock, reset, and future protocols, policy uses
method-specific bounds rather than pretending they are MMIO:

- method allowlist;
- transfer length;
- bus address or chip-select restrictions where applicable;
- configuration ranges;
- timeout/deadline;
- permitted state changes; and
- reset/cleanup behavior.

#### 9.6 Policy generation

When policy is expressed in JSON5 or another checked-in format, generate:

1. a target-native immutable table; and
2. a canonical manifest returned in `Describe`.

Generation rejects duplicate IDs/names, overflow, ambiguous overlap, invalid
masks, unreachable cleanup, nondeterministic ordering, and limits above FIDL
maxima.

Hash the canonical manifest bytes. Evidence records the digest.

### 10. Session allowlist

#### 10.1 Purpose

The session allowlist is derived from the canonical host plan after permission
resolution. It prevents a stale or buggy client from accessing anything beyond
the frozen run, even though it is not cryptographic proof of approval.

#### 10.2 Required identity

Opening a session includes:

- run and case IDs;
- plan digest;
- host tool version;
- expected boot ID;
- expected proxy generation;
- expected resource-description digest;
- expected target-policy digest;
- read-only or mutating mode;
- exact allowed operations or access rules; and
- bounded plan limits.

The proxy rejects a stale identity before access.

#### 10.3 Narrowing validation

Before opening the session, the proxy validates the entire allowlist against
the target ceiling. A rejected allowlist creates no session and performs no
hardware operation.

A read-only session can never issue a write even if a write appears in target
policy. A mutating session additionally requires an exclusive mutation lease.

### 11. Wire contract

The final FIDL syntax must match the selected Fuchsia revision. The semantic
contract below is normative. The V1 contract is implemented as the
`fuchsia.driver.lab` library with the `Proxy` (`Describe`,
`OpenSession`) and `Session` (`Read32`, `Snapshot`, `ReadAudit`) protocols.

#### 11.1 Limits

The protocol declares compile-time bounds for:

- resource names and device IDs;
- resource count;
- operation count;
- snapshot item count;
- protocol transaction size;
- audit page size;
- detail strings;
- session allowlist rules; and
- total encoded response size.

Runtime target limits may be smaller.

#### 11.2 Proxy description

`Describe` returns:

- protocol major/minor;
- proxy implementation version;
- proxy instance/generation;
- boot ID;
- node identity and topology generation;
- takeover identity when applicable (reserved; populated only in phase 2);
- target policy name/digest;
- canonical resource descriptions and digest;
- supported resource and operation kinds;
- runtime limits;
- audit capacity; and
- fault-isolation/session capabilities.

#### 11.3 Session creation

`OpenSession` accepts run context, session mode, identity expectations, and the
exact allowlist. It returns a session channel and session ID.

Read-only sessions may coexist when safe. Exactly one mutating session exists
per proxy instance. Target-local sequences are serialized initially, including
read-only sequences whose interleaving would make evidence ambiguous.

The allowlist is fixed for the session's lifetime in V1. Expanding consent
requires closing the session and reopening it with the expanded allowlist; a
narrowing-only `ExtendAllowlist` operation is a deferred open decision
(section 26).

#### 11.4 MMIO operations

The initial scalar MMIO methods are:

- `Read32(resource_id, offset)`;
- `Write32(resource_id, offset, value, write_mask, precondition, readback)`;
- `Snapshot(items)`; and
- `Poll32(resource_id, offset, expected, mask, interval, timeout)`.

Additional widths require protocol and policy revision. Offsets are bytes from
the start of a logical resource.

#### 11.5 Ordered sequence

`ExecuteSequence` accepts a bounded ordered list containing:

- resource-specific scalar operations;
- delay;
- memory barrier; and
- only those protocol transactions whose exact semantics are represented.

Every entry identifies resource kind, resource ID, and operation-specific
fields. Unknown operations produce `ZX_ERR_NOT_SUPPORTED`.

The sequence is not atomic. Whole-sequence structural/policy validation occurs
before the first hardware operation. Hardware preconditions are evaluated at
execution time.

#### 11.6 Resource-specific endpoints

Where practical, a session should open a typed endpoint for a standard
protocol-backed resource. The endpoint:

- preserves generated FIDL request/response types;
- is bound to the session identity;
- enforces the same target policy and allowlist;
- writes the same audit records; and
- observes the same deadlines and cancellation.

If an exact production protocol cannot represent necessary debug policy or
audit behavior, define a narrowly shaped adapter and document the difference.

#### 11.7 Interrupt observation

`WaitForInterrupt(resource_id, after_sequence, timeout)` is a bounded hanging
request. The proxy retains and acknowledges the interrupt locally.

The report includes:

- interrupt resource ID;
- event sequence;
- total count;
- target timestamp; and
- coalesced/overflow information.

The initial protocol observes interrupts only. It does not let the host change
polarity, routing, or masking.

#### 11.8 Audit

`ReadAudit(cursor, limit)` returns a bounded page and reports the oldest
available sequence so the host can detect gaps.

#### 11.9 Versioning

- Increment minor for optional compatible fields and safely rejectable flexible
  operations.
- Increment major for changed operation meaning, policy guarantees, session
  semantics, or evidence interpretation.
- Never reuse an ordinal with different meaning.
- The host requires an exact major version.
- Golden descriptions and compatibility matrices cover every supported major.

### 12. MMIO semantics

#### 12.1 Read validation

Immediately before every read:

1. verify session and proxy generation;
2. find the resource;
3. require supported width and alignment;
4. check `offset + width` for overflow;
5. require the access within logical size;
6. require the access within actual mapped size;
7. enforce target read ceiling/hard denies;
8. match the exact session allowlist;
9. enforce access-class count/rate/deadline limits; and
10. perform one volatile read.

Rejected accesses are audited without touching hardware.

#### 12.2 Masked writes

For an ordinary 32-bit masked write:

```text
policy_mask = immutable target mask
requested   = request.write_mask

require requested != 0
require (requested & ~policy_mask) == 0

if requested covers the full register width
   and no precondition is configured or requested:
    volatile_write32(request.value)
else:
    require read-modify-write valid for this policy entry
    before = volatile_read32()
    require precondition if configured
    after = (before & ~requested) | (request.value & requested)
    volatile_write32(after)

apply required barrier
read back configured stable bits if required
```

The before-read in the read-modify-write path is itself a hardware read with
potential side effects; it is audited, and it is permitted only where the
policy entry declares read-modify-write valid. Write-only registers must be
declared full-mask, non-read-modify-write, and without preconditions.

Reject rather than silently narrow a requested mask.

#### 12.3 Polls and delays

Polls use asynchronous timers on the driver dispatcher. They never busy-wait
and never block the dispatcher with sleep.

Dispatcher timers provide millisecond-class scheduling precision. V1 makes no
sub-millisecond guarantee for delays, polls, or inter-operation gaps; whether
ceiling-bounded short spin-delays are permitted for microsecond-class
sequencing is an open decision (section 26). Declared timing precision is
part of capability reporting.

The proxy enforces:

- maximum interval frequency;
- maximum timeout;
- maximum iteration count;
- sequence wall-clock deadline; and
- cancellation on session close or stop.

#### 12.4 Barriers

Use the architecture/platform MMIO primitives from the selected Fuchsia tree.
A barrier orders accesses; it does not prove device-internal completion.

#### 12.5 Snapshot

Validate every item before the first read. The snapshot is an ordered series of
reads, not an atomic hardware snapshot. Each value includes an individual
timestamp, status, and audit sequence.

### 13. Execution and dispatcher model

Start with one synchronized dispatcher for:

- FIDL requests;
- session state;
- execution state machines;
- timers;
- interrupt callbacks;
- audit ordering; and
- driver stop.

Use an asynchronous sequence state machine:

```text
ValidateWholeSequence
    |
    v
BeginOperation
    +-- immediate operation -> Record -> next
    +-- delay -> arm timer -> resume -> Record -> next
    +-- poll -> read -> matched/timeout/arm interval
    +-- protocol wait -> async completion -> Record -> next
```

A second target-local sequence receives `ZX_ERR_SHOULD_WAIT` until safe
concurrency semantics are designed.

On failure:

- stop when `stop_on_error` is true;
- continue only for explicitly recoverable errors when it is false;
- never continue after stop, peer closure, invalid backend state, or a
  policy-classified fatal error; and
- initially require `stop_on_error = true` for every mutating sequence.

### 14. Core implementation boundaries

Use narrow testable modules:

```text
ProxyDriver
    lifecycle, outgoing service, instance identity

ResourceProvider
    node/parent-specific acquisition

HardwareBackend
    MMIO, barriers, clock, and injected protocol operations

AccessPolicy
    actual bounds + target ceiling + session allowlist

Executor
    scalar operations and asynchronous sequences

SessionServer
    run context, allowlist, leases, cancellation

ProtocolResourceAdapter
    GPIO/SPI/I2C/serial/clock/reset endpoints

InterruptTracker
    async waits, counters, acknowledgment

AuditRing
    bounded ordered target evidence
```

FIDL conversion stays at service boundaries. Policy and executor tests run
without a component realm: the pure-logic modules form the
`lab_proxy_core` crate, which builds for both the target and the build
host so their tests run without an emulator.

### 15. Driver start

`Start` must:

1. read immutable engineering configuration;
2. load/generate the target ceiling;
3. connect to the node's parent-native resource provider;
4. enumerate offered resources;
5. validate policy against actual resources;
6. map permitted MMIO locally;
7. obtain and register permitted interrupts;
8. connect permitted protocol-backed resources;
9. derive boot/proxy/resource identities and digests;
10. create FIDL binding groups;
11. publish the proxy service through the Driver Framework route expected by
    the Driver Manager control plane; and
12. emit one structured ready event.

Start performs no experimental write. Avoid reads with possible side effects.
An identity read during start is allowed only when explicitly declared safe
and audited.

Start fails if a required resource is absent, undersized, ambiguous, or
incompatible with policy.

### 16. Sessions and leases

A session owns:

- run context;
- exact allowlist;
- read-only/mutating mode;
- session ID;
- pending timers/waits;
- sequence state; and
- audit cursor metadata.

On session channel close:

- cancel timers and waits;
- stop new operations;
- release the mutation lease;
- audit session closure;
- run only explicitly configured fail-safe cleanup; and
- finish within a bounded deadline.

Cleanup is fallible and audited. A resource with unknown safe cleanup has no
automatic cleanup and relies on normal driver rebind, target reset, or
out-of-band recovery.

### 17. Interrupt handling

For every interrupt callback:

1. capture target monotonic time;
2. increment count and sequence;
3. calculate coalescing information where available;
4. satisfy eligible waiters;
5. append an event/audit entry; and
6. acknowledge the interrupt on every path.

Use virtual interrupts in tests. An interrupt storm must not allocate
unbounded memory or starve stop/restoration.

### 18. Audit ring

The fixed-capacity ring assigns monotonically increasing 64-bit sequence
numbers. Wrap overwrites old entries but does not reset sequence.

The ring is in-memory: a driver-host crash loses undrained entries. Host
workflows must therefore drain audit incrementally during mutating plans
rather than only at run end.

Each entry contains:

- target boot and proxy generation;
- session and run context;
- resource identity;
- operation index and normalized operation;
- allowlist match/grant reference when provided;
- target policy decision;
- status;
- applicable before/after/value data;
- start/completion or event timestamp; and
- cleanup/failure classification.

Never audit pointer values, physical addresses, channel handles, or unbounded
caller strings.

### 19. Stop

`PrepareStop` must:

1. reject new sessions;
2. stop accepting operations;
3. cancel sequence timers and protocol waits;
4. cancel interrupt waits;
5. finish in-flight callbacks;
6. close FIDL bindings with an epitaph;
7. run only bounded configured cleanup;
8. stop interrupt handlers;
9. release mappings/resources; and
10. complete stop within the Driver Manager stop deadline.

Every pending call terminates with a result, epitaph, cancellation, or bounded
host timeout.

### 20. Error model

Use typed application errors and Zircon status consistently:

- invalid/misaligned input;
- out of resource bounds;
- target ceiling denial;
- session allowlist denial;
- stale boot/proxy/resource identity;
- failed precondition;
- unsupported resource/method;
- timeout;
- cancellation;
- mutation lease contention;
- partial execution; and
- fatal backend state.

Machine-parsed information must not exist only in a human detail string.

Execution reports contain:

- sequence/batch ID;
- first and last audit sequence;
- result for every operation that began;
- stopped-on-error flag;
- partial-completion classification;
- fatal/nonfatal classification; and
- proxy health after execution.

### 21. Source layout

Exact paths may follow repository conventions:

```text
src/devices/driver-lab/
    BUILD.gn
    meta/
    fidl/
        driver_lab.fidl          fuchsia.driver.lab wire contract; moves to
                                 an internal-category SDK library once the
                                 contract stabilizes
    src/
        lib.rs                   driver lifecycle and service publication
        server.rs                FIDL session server
        core.rs                  host-testable core crate root
        access_policy.rs
        audit_ring.rs
        digest.rs
        executor.rs
        hardware_backend.rs
        session.rs
        resource_provider.rs           planned: parent adapters (P0/P1)
        protocol_resource_adapter.rs   planned (P4)
        interrupt_tracker.rs           planned (P5)
    policy/                            planned: schema and generation
    bind/

src/devices/bin/driver_manager/
    ... engineering takeover integration (phase 2) ...
```

The proxy is implemented in Rust on the DFv2 Rust driver bindings. Where the
Rust driver-framework, MMIO, or interrupt bindings lag their C++ equivalents
in the selected revision, the gap is a feasibility-spike finding to resolve
upstream, not a reason to switch languages. The wire contract and host
programming model remain language-neutral.

### 22. Build and configuration

Create separate targets for:

- FIDL libraries and generated bindings;
- policy schema/generator;
- pure policy/executor library;
- proxy driver/component/package;
- unit and integration tests;
- Driver Manager takeover integration (phase 2);
- host bindings and compatibility goldens;
- engineering assembly inclusion; and
- production-absence verification.

Structured configuration may reduce:

- enabled state;
- audit capacity;
- log verbosity;
- maximum operation count;
- maximum deadlines; and
- automatic restoration timeout (phase 2).

It may not widen resource or write policy.

### 23. Testing

#### 23.1 Policy tests

Cover:

- known/unknown resources;
- logical versus actual sizes;
- digest determinism;
- hard-denied reads;
- unknown read with and without exact allowlist;
- width/alignment;
- overflow;
- one-shot versus poll permission;
- target limit narrowing;
- read-only versus mutating session;
- exact write offset/mask;
- precondition/readback requirements;
- protocol method and transaction limits; and
- cleanup validation.

#### 23.2 Executor tests

Use fake MMIO, fake clocks, fake timers, and fake protocol resources. Cover:

- scalar read;
- snapshot prevalidation;
- masked write calculation;
- rejected mask performs no write;
- failed precondition;
- readback success/mismatch;
- immediate and delayed poll success;
- poll timeout;
- barriers and ordering;
- protocol transaction;
- stop-on-error partial result;
- cancellation;
- audit/result correspondence; and
- concurrent sequence rejection.

#### 23.3 Driver realm tests

Provide fake parent resources and test:

- platform-like and PCI-like providers;
- resource enumeration;
- missing/undersized resources;
- mapping and device cache policy;
- service publication;
- session opening;
- virtual interrupts;
- driver stop;
- pending request cancellation; and
- no resource handles returned to the host.

#### 23.4 Activation integration tests

With a synthetic unclaimed node:

1. register and bind the proxy through existing mechanisms;
2. verify identity, generation, and digest reporting;
3. execute a read-only plan;
4. stop the proxy; and
5. verify the node is again unclaimed.

Inject failure at every transition. Managed-takeover integration tests are
specified in phase 2.

#### 23.5 Interrupt tests

Verify count/sequence, retained events, future waits, timeout, coalescing,
multiple observers, close cancellation, stop cancellation, acknowledgment,
storm bounding, and stop/teardown responsiveness.

#### 23.6 Security-negative tests

Verify rejection of:

- physical address input;
- unknown resource;
- resource size boundary/overflow;
- unsupported width;
- hard-denied read;
- read absent from session allowlist;
- one-shot grant used for poll;
- write absent from immutable target policy;
- excessive write mask;
- missing precondition;
- oversized operation vectors or transactions;
- excessive timeout/delay;
- mutation from read-only session;
- second mutating session;
- stale identity/digest;
- arbitrary driver URL or protocol path;
- concurrent ownership with normal driver; and
- proxy/control artifacts in a production image.

#### 23.7 Fault injection

Cover:

| Fault | Required result |
|---|---|
| Peer closes before operation | no access; transport failure |
| Peer closes after write | partial/unknown result remains visible |
| Stop during delay/poll | bounded cancellation |
| Driver host crashes | proxy generation changes; session becomes stale |
| Target reboot | boot identity changes; no replay |
| Audit wrap | explicit gap reported |
| Interrupt storm | bounded memory and responsive stop |
| Protocol backend hangs | target/host deadline and recovery path |
| Cleanup fails | audited failure; no success claim |

### 24. Milestones

#### Milestone P0: feasibility

- validate proxy bind to an unclaimed node via existing registration and bind
  mechanisms;
- validate Rust DFv2 driver support for MMIO mapping, interrupt handling,
  synchronized-dispatcher timers, and service publication;
- classify candidate parent resource providers as channel-transport or
  driver-transport-only in the selected revision;
- validate the first parent resource provider;
- validate isolated host placement;
- define the engineering/production assembly boundary with platform assembly
  owners; and
- (phase-2 gate) review node-scoped force-bind, node persistence across
  normal-driver unbind, and restoration with the driver-framework team.

#### Milestone P1: read-only proxy

- `Describe`;
- immutable ceiling;
- session allowlist;
- named MMIO mapping;
- `Read32` and snapshot;
- audit; and
- driver realm tests.

Exit: only an exact allowed read inside an offered resource reaches fake MMIO.

#### Milestone P2: activation integration

- proxy registration, bind, and service routing on an unclaimed synthetic
  node;
- generation identity;
- bounded stop; and
- verified teardown back to an unclaimed node.

Exit: synthetic activation and teardown succeed and every injected transition
failure is visible. (Managed-takeover integration is the corresponding
phase 2 milestone.)

#### Milestone P3: bounded sequence and mutation

- delay, poll, and barrier;
- exact masked writes;
- precondition/readback;
- mutation lease;
- cleanup semantics;
- partial completion; and
- expanded fault tests.

Exit: no out-of-policy bit can be written and no failed sequence is called
atomic.

#### Milestone P4: protocol resources

- representative GPIO and SPI or I2C adapters;
- production-shaped typed endpoints;
- method-specific policy and audit;
- heterogeneous target-local sequence; and
- translation-fidelity documentation.

Exit: host Python can perform equivalent resource operations through direct and
proxy modes without flattening their semantics.

#### Milestone P5: interrupts and physical validation

- interrupt observation;
- virtual tests;
- selected hardware-safe read validation;
- selected reviewed mutation;
- driver-host failure drill; and
- out-of-band recovery drill.

Exit: hardware runs preserve evidence and return the target to a known
operational state.

### 25. Definition of done

The phase 1 proxy is ready when:

- production images verifiably do not offer it;
- it is present but inactive in selected engineering images;
- it binds without an arbitrary host-supplied driver URL;
- it binds only to nodes with no bound driver;
- it receives only resources offered to its node;
- no host API accepts a physical address or raw target resource handle;
- actual bounds, target ceiling, and session allowlist are all enforced;
- unknown reads require an exact session grant;
- writes require immutable exact target policy;
- operations and waits are bounded;
- target-local execution never blocks the dispatcher;
- partial completion is represented honestly;
- audit covers attempted and rejected operations;
- driver stop cancels all pending work;
- teardown verifiably returns the node to an unclaimed state;
- synthetic, security-negative, fault-injection, and selected physical tests
  pass;
- the wire contract remains usable from the companion host tooling
  (`//tools/driver-lab/SPEC.md`); and
- the wire contract carries the reserved takeover fields so phase 2 requires
  no major version change.

### 26. Open decisions

Before implementation, resolve:

1. The first parent resource-provider adapter.
2. How generic engineering read ceilings are supplied without unsafe global
   defaults.
3. Which offsets/ranges must be hard-denied even with operator consent.
4. The initial write-policy authoring and review process.
5. Whether standard FIDL protocols can be served unchanged while preserving
   session policy/audit.
6. The Rust driver-framework, MMIO, interrupt, and timer APIs available in
   the selected Fuchsia revision, and any binding gaps requiring upstream
   work.
7. Whether a narrowing-only `ExtendAllowlist` session operation replaces V1's
   reopen-per-grant behavior.
8. Whether ceiling-bounded short spin-delays are permitted for
   microsecond-class sequencing.

Takeover-related decisions (force-bind and restore APIs, node-class
persistence, restoration-state persistence, driver-host restart during
takeover, subtree teardown) live in the Phase 2 section of this document.

### 27. Primary references

- [Driver communication and services](https://fuchsia.dev/fuchsia-src/concepts/drivers/driver_communication)
- [Mapping device memory in a driver](https://fuchsia.dev/fuchsia-src/concepts/drivers/mapping-a-devices-memory-in-a-driver)
- [Using hardware resources in a driver](https://fuchsia.dev/fuchsia-src/development/drivers/tutorials/sdk_build_driver/hardware-resources)
- [Handling interrupts in a driver](https://fuchsia.dev/fuchsia-src/development/drivers/developer_guide/handle-interrupts-in-a-driver)
- [Driver runner and driver hosts](https://fuchsia.dev/fuchsia-src/concepts/components/v2/driver_runner)
- [Fuchsia registers driver](https://fuchsia.dev/fuchsia-src/development/drivers/driver_guides/registers/overview)
- [Structured configuration](https://fuchsia.dev/fuchsia-src/development/components/configuration/structured_config)
- [Build configuration and product assembly](https://fuchsia.dev/fuchsia-src/development/build/software_assembly/build_configuration)

## Phase 2: managed takeover

### Implementation status

No phase 2 changesets are scheduled: phase 2 is deferred pending
phase 1 completion and driver-framework team review of the takeover
mechanism (section 2 below). When changesets are scheduled they will
be listed here with the same global numbering used in phase 1.

Phase: 2 -- existing-driver exploration through managed takeover. Deferred
pending phase 1 completion and driver-framework team review of the takeover
mechanism.

Extends: Phase 1 of this document. All phase 1 requirements apply unchanged;
this section adds the Driver Manager takeover protocol's target-side behavior
and the proxy's takeover-specific obligations.

Companion specification: host tooling at `//tools/driver-lab/SPEC.md`

### 1. Purpose

Phase 2 lets Driver Manager temporarily replace a node's normal driver with
the proxy and later restore it. The proxy implementation is the phase 1
driver unchanged except where stated here: the same policy, sessions,
executor, audit, and wire contract serve both activation paths, and the
phase 1 contract already reserves the takeover-identity fields.

### 2. Prerequisites

Phase 2 implementation must not begin until:

1. phase 1 is complete and in use;
2. the driver-framework team has reviewed the node-scoped force-bind and
   restoration design; and
3. the open decisions in section 8 of this phase have accepted answers.

### 3. Driver Manager responsibilities

Before proxy start, Driver Manager is responsible for:

- checking staleness expectations (expected bound-driver URL and driver-host
  koid, or an explicit topology generation if one is provided);
- acquiring the exclusive takeover lease;
- orderly stop and unbind of the original driver, including its descendant
  subtree;
- recording the original driver's identity for restoration (never taken from
  plan JSON);
- confirming that the device node persists; and
- binding the configured proxy through the new node-scoped force-bind
  mechanism.

The force-bind mechanism is new Driver Manager work: it must be node-scoped,
and the existing `DisableDriver` (global by URL) is unsuitable. It must be
validated in the selected Driver Framework revision.

After proxy access ends, Driver Manager stops and unbinds the proxy, rebinds
the recorded original driver, and verifies its identity and generation.
Restoration failure is recorded as a queryable terminal state; Driver Manager
never reports it as success.

On host-channel loss, Driver Manager stops accepting new work and attempts
the configured bounded restoration policy (defined jointly with the host
tooling specification at `//tools/driver-lab/SPEC.md`).

The takeover integration follows Driver Manager's existing implementation
language and conventions (C++); the proxy itself remains the phase 1 Rust
driver.

### 4. Proxy obligations during takeover

- The proxy receives run/takeover identity in immutable start metadata or
  during its first control handshake, and populates the reserved takeover
  fields of `Describe`.
- The proxy does not restore the original driver itself.
- `PrepareStop` must complete within the Driver Manager takeover deadline so
  restoration is never blocked by pending experimental work.
- An interrupt storm or hung protocol backend must not starve stop or
  restoration.

### 5. Subtree teardown

Unbinding a non-leaf node tears down descendant nodes and stops their
drivers. How takeover eligibility, consent display, and restoration
verification account for the affected subtree -- including descendants'
power-framework participation -- is an open design area (section 8 of this
phase). Takeover of a node with safety-relevant descendants must be
ineligible until that design exists.

### 6. Production absence

Production images must not offer the takeover protocol, and the build must
verify this. The verification mechanism (Driver Manager binary variant, or
config gating with capability-route verification) is owned by platform
assembly; this specification requires only that the chosen mechanism be
verifiable at build time.

### 7. Testing and milestone

Takeover integration tests, with a synthetic normal driver and persistent
node:

1. bind normal driver;
2. request takeover;
3. stop/unbind it;
4. bind proxy;
5. execute read-only plan;
6. stop proxy;
7. rebind normal driver; and
8. verify generation changes and restoration reporting.

Inject failure at every transition. The Driver Manager portion is tested
jointly with the host tooling specification (`//tools/driver-lab/SPEC.md`).

Additional fault-injection rows beyond the phase 1 table:

| Fault | Required result |
|---|---|
| Host disconnect mid-takeover | bounded restoration; queryable outcome |
| Driver stop failure | takeover aborted; state recorded |
| Original-driver rebind failure | terminal restoration failure; out-of-band recovery |

Security-negative additions: rejection of a takeover request carrying an
arbitrary driver URL, and of concurrent ownership with the normal driver.

Milestone exit: synthetic replacement/restoration succeeds and every injected
transition failure is visible.

### 8. Open decisions

Before implementation, resolve:

1. The exact node-scoped force-bind and restore APIs.
2. Which node classes persist safely after their driver is unbound.
3. Subtree teardown: descendant enumeration, eligibility, consent display,
   restoration verification, and power-framework element handling.
4. How restoration timeout and Driver Manager recovery state are persisted.
5. Whether automatic driver-host restart is disabled during an active
   takeover.
6. Takeover-protocol packaging (binary variant versus config gating) and its
   build-time verification, owned by platform assembly.
7. Whether nodes whose parents offer resources only over driver-transport
   (`fdf`) FIDL remain ineligible for takeover -- the isolated-host
   requirement implies they are -- or a colocated proxy mode is introduced,
   with its Rust `fdf`-transport binding implications.
