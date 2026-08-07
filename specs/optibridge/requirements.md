# OptiBridge Firmware Requirements

## Scope

This specification describes the CH32V203G6U6 OptiBridge firmware's bounded
I2C action surface. It implements Reset, Load BPF, Read BPF map, and Write
BPF map, exposes one remaining action stub, and exposes startup liveness
through Read Status.

**KNOWN:** BPF verification beyond the startup probe, optical I/O, calibration,
interrupts, and a general status-buffer API are not implemented and are out of
scope. This specification adds the flash-resident BPF image loader,
image-CRC query, Read and Write BPF map surfaces, two array-map helpers, and
timer-driven BPF event execution.

## Requirements

### REQ-OPT-FW-001: Platform and generated HAL

The firmware **MUST** target CH32V203G6U6 and consume the externally generated
HardwareAbstractionIR HAL. It **MUST NOT** vendor HAL source code.

The HAL generator input and source revision **MUST** be pinned to
`60e0de038210008018d0168f45854d113a5964cc` and regenerate the CH32V203G6U6
HAL from that pin.

### REQ-OPT-FW-002: Startup and I2C electrical configuration

Startup **MUST** configure the 48 MHz USB FS device clock and Embassy time
runtime, enable and release-reset GPIOB and I2C1 slave resources, and set
PB6/SCL and PB7/SDA high before configuring them as 50 MHz alternate-function
open-drain outputs on I2C1's default route.

The firmware **MUST** configure I2C1 as a slave with own seven-bit address
`0x42`.

### REQ-OPT-FW-003: Packet-scoped request parsing

The firmware **MUST** process each completed I2C receive capture independently.
It **MUST NOT** carry a partial frame from one capture to a later capture.

An incomplete capture **MUST** replace the response slot with an empty packet.

### REQ-OPT-FW-004: Multiple frames and malformed input

When one non-truncated I2C receive capture contains multiple complete valid
frames, the firmware **MUST** dispatch only the last complete frame.

On a parser error or a truncated capture, the firmware **MUST** abandon the
remaining bytes and replace the response slot with an empty packet.

### REQ-OPT-FW-005: Shared frame format

Requests and responses **MUST** use the shared frame format:

```text
byte 0: magic = 0xA5
byte 1: command (request) or status (response)
byte 2: payload length, 0 through 16
byte 3: sequence
byte 4: flags
bytes 5..: payload
```

Request flags are reserved and **MUST** be zero. Responses **MUST** set flags
to zero.

### REQ-OPT-FW-006: I2C error behavior

For every completed I2C receive capture, the firmware **MUST** replace the
single response slot with either one bounded protocol response or an empty
packet. A completed malformed, incomplete, or truncated capture **MUST** use
an empty packet.

Valid Reset is the exception: it **MUST** initiate reset before queuing a
response.

Low-level I2C bus-error paths that do not deliver a completed capture are
deferred; this change **MUST NOT** introduce bus recovery or a response-slot
clear API outside the generated HAL.

### REQ-OPT-FW-007: Sonde size probe

Startup **MUST** execute the fixed two-instruction Sonde BPF program that
returns `42`. If the probe fails or produces another value, firmware **MUST**
panic. This probe **MUST NOT** be represented as a BPF program loader or
runtime feature.

### REQ-OPT-FW-008: Resource constraints

The firmware **MUST** be `no_std` and allocation-free. Its release `text +
data` flash use **MUST NOT** exceed 24,576 bytes.

The firmware image **MUST** link entirely below flash offset `0x6000`, reserving
`0x6000..=0x7fff` exclusively for BPF image storage. All static RAM, runtime
stack, 1,024-byte map backing store, and Sonde interpreter execution stack
**MUST** fit in the 10,240-byte RAM region. The final link image **MUST**
reserve at least 4,096 bytes of stack for Sonde execution.

### REQ-OPT-FW-009: Repeated transaction availability

After startup, the I2C slave **MUST** accept consecutive master
write/read-response cycles at address `0x42` without an OptiBridge reset.
After a master terminates a response read with STOP or NACK, the slave **MUST**
remain available for the next write.

### REQ-OPT-FW-010: Bounded ISR dispatch

The firmware **MUST** register the generated I2C RX-packet ISR dispatcher with
a static receive buffer of `MAX_FRAME` bytes. Its callback **MUST NOT**
allocate, await, or use unbounded storage. Responses **MUST NOT** exceed
`MAX_FRAME`, which fits within the generated 32-byte single response slot.

### REQ-OPT-FW-011: Dispatch ordering

For a valid Read Status capture, the firmware **MUST** consume the stored
status before queuing the response. Invalid requests **MUST NOT** consume the
stored status.

### REQ-OPT-FW-012: Empty response behavior

An empty response slot is not a protocol response. A master read after an
empty packet **MUST NOT** be interpreted as a shared frame; the generated HAL
may transmit zero filler bytes.

### REQ-OPT-FW-013: Single-slot response ordering

The I2C master **MUST** complete its response-read transaction before writing
the next OptiBridge command. The firmware **MUST** maintain at most one queued
response. If a later valid command arrives before the prior response is read,
the later response **MUST** replace the unread response. The firmware **MUST
NOT** introduce a multi-entry response queue.

### REQ-OPT-ACT-001: Action command identifiers

`CMD_ALIVE` **MUST NOT** be present. The shared protocol **MUST** define the
following action commands:

| Command | Value |
| --- | ---: |
| `CMD_RESET` | `0x01` |
| `CMD_LOAD_BPF` | `0x02` |
| `CMD_START_BPF` | `0x03` |
| `CMD_READ_BPF_MAP` | `0x04` |
| `CMD_WRITE_BPF_MAP` | `0x05` |
| `CMD_READ_STATUS` | `0x06` |
| `CMD_QUERY_BPF_CRC` | `0x07` |

### REQ-OPT-ACT-002: Status storage

Startup **MUST** initialize fixed-capacity, allocation-free status storage with
the ASCII message `ready`. Status storage **MUST** retain only its newest
message and its message **MUST NOT** exceed 16 bytes.

### REQ-OPT-ACT-003: Read Status

`CMD_READ_STATUS` **MUST** require zero flags and zero payload. It **MUST**
return `STATUS_OK`, retain the request sequence, and include the newest status
while removing it from status storage. When no status is available, it **MUST**
return `STATUS_OK` with an empty payload.

The initial response payload **MUST** be ASCII `ready`.

### REQ-OPT-ACT-004: Start BPF request validation

`CMD_START_BPF` **MUST** require zero flags and zero payload. Nonzero flags
**MUST** return `STATUS_BAD_FLAGS`; a nonempty payload **MUST** return
`STATUS_BAD_LENGTH`. Invalid requests **MUST NOT** execute BPF or mutate
runtime state.

### REQ-OPT-ACT-005: Start BPF execution

When a validated committed image exists, a valid `CMD_START_BPF` request **MUST**
invoke that image exactly once with an empty context and the approved array-map
helpers from `REQ-OPT-BPF-013`. The interpreter return value `0` **MUST**
produce a status-only `STATUS_OK` response; a nonzero return or interpreter
failure **MUST** produce a status-only `STATUS_BAD_COMMAND` response.
Responses **MUST** retain the request sequence.

When no validated committed image exists, Start **MUST** return
`STATUS_NO_PROGRAM` without invoking the interpreter.

### REQ-OPT-BPF-010: Start BPF runtime state

After a successful Start, the image **MUST** enter the running state. A
subsequent Start or Load BPF request **MUST** return `STATUS_BAD_STATE` until
reset. Query BPF CRC **MUST** remain read-only and return the committed CRC;
map reads and writes **MAY** continue to use the committed map layout.

Start **MUST NOT** mutate flash contents, map descriptor layout, or status
storage. Reset **MUST** clear volatile running state while preserving the
committed image and CRC.

### REQ-OPT-ACT-006: Unknown commands

Commands outside the seven defined action values **MUST** return status-only
`STATUS_BAD_COMMAND` and retain the request sequence.

### REQ-OPT-ACT-007: Deferred event execution semantics

Timer-triggered BPF invocation **MAY** be introduced only through the approved
timer helpers and event context requirements below. GPIO, DMA, optical, and
other event sources remain out of scope. Event-triggered execution **MUST NOT**
change the existing I2C command surface or introduce recursive interpreter
entry.

### REQ-OPT-ACT-008: Immediate Reset

`CMD_RESET` **MUST** require zero flags and zero payload. A valid request
**MUST** request generated HAL `interrupt::system_reset()` after its I2C ISR
returns and **MUST NOT** queue a protocol response. A master **MUST NOT** read
a target response after a valid Reset request and **MUST** wait at least one
second before its next I2C request.

### REQ-OPT-ACT-009: Invalid Reset

Reset with nonzero flags **MUST** return `STATUS_BAD_FLAGS`. Reset with zero
flags and nonempty payload **MUST** return `STATUS_BAD_LENGTH`. Invalid Reset
requests **MUST NOT** reset the MCU or consume status.

### REQ-OPT-FW-014: Reset reinitialization

The firmware **MUST** use generated HAL `interrupt::system_reset()` rather
than direct PFIC MMIO. After restart, it **MUST** restore I2C address `0x42`
and the initial `ready` status. It **MUST NOT** add reset acknowledgments,
delay-based reset handling, allocation, or a reset-recovery subsystem.

### REQ-OPT-BPF-001: Reserved image storage

The firmware **MUST** reserve the two 4,096-byte flash pages at offsets
`0x6000..=0x7fff` for a single BPF image and **MUST NOT** link executable or
read-only firmware data into that range. A valid image **MUST** have a
16-byte header followed by canonical image bytes: bytecode first, then map
descriptors in map-index order.

The 16-byte header **MUST** encode `OBPF` magic at bytes `0..4`, format
version `1` at byte `4`, map count at byte `5`, bytecode length as a
little-endian `u16` at bytes `6..8`, CRC-32/ISO-HDLC as a little-endian `u32`
at bytes `8..12`, commit marker as a little-endian `u16` at bytes `12..14`,
and erased reserved bytes at `14..16`. CRC-32 **MUST** use polynomial
`0x04C11DB7`, reflected input/output, initial value `0xFFFFFFFF`, and final
XOR `0xFFFFFFFF`. The commit marker **MUST** be `0xFFFF` until all image bytes
and the CRC have been validated, then be programmed to `0x0000` last.
Firmware **MUST** treat an erased, malformed, or CRC-mismatched header as no
image.

### REQ-OPT-BPF-002: Image bounds and map definitions

Bytecode **MUST** be nonempty, eight-byte aligned, and no more than 7,680
bytes (960 BPF instructions). An image **MUST** define no more than eight
maps. Each canonical map descriptor **MUST** be 16 little-endian bytes:
`map_type`, `key_size`, `value_size`, and `max_entries`, each a `u32`.

Only Sonde `BPF_MAP_TYPE_ARRAY` (`map_type = 1`) with `key_size = 4` **MUST**
be accepted. Every definition **MUST** have nonzero `value_size` and
`max_entries`; the checked sum of `value_size * max_entries` across all maps
**MUST NOT** exceed 1,024 bytes. Definitions that overflow, exceed image
storage, or violate these constraints **MUST** be rejected.

### REQ-OPT-BPF-003: Load BPF command format

`CMD_LOAD_BPF` **MUST** require zero request flags. Its first payload byte
**MUST** select one of these operations:

| Operation | Value | Payload after operation byte |
| --- | ---: | --- |
| Begin | `0x00` | bytecode length (`u16` little-endian), map count (`u8`), expected CRC-32 (`u32` little-endian) |
| Data | `0x01` | canonical-image offset (`u16` little-endian), 2 through 12 data bytes |
| Finalize | `0x02` | Empty |

Data-byte counts **MUST** be even. Begin **MUST** validate declared bounds
before accepting a transfer. Data offsets **MUST** equal the next expected
canonical-image offset; duplicate, skipped, or overlapping data **MUST** be
rejected. Finalize **MUST** require exactly all declared image bytes.

### REQ-OPT-BPF-004: Flash write discipline

The loader **MUST** erase each required 4,096-byte reserved flash page at
most once per load attempt, before programming its first image byte. It
**MUST** program image data only in ascending, two-byte-aligned order and
**MUST NOT** erase a page per I2C fragment or rewrite programmed bytes.

The loader **MUST NOT** require a page-sized RAM buffer. Erase, program,
header validation, and CRC calculation **MUST NOT** run in the I2C callback.

### REQ-OPT-BPF-005: Deferred command completion

The I2C callback **MUST** copy at most one valid Load BPF request into
fixed-capacity pending state and return `STATUS_OK` only to acknowledge
acceptance. The main loop **MUST** perform the associated flash operation.
While an operation is pending, further Load BPF requests **MUST** return
`STATUS_BUSY`; such requests **MUST NOT** change loader state. A flash
failure **MUST** be retained until the next Begin or reset and exposed by
`CMD_QUERY_BPF_CRC`.

### REQ-OPT-BPF-006: Load state and reset

Before BPF execution begins, Begin **MAY** replace a previously committed
image. A failed or incomplete attempt **MUST NOT** be marked committed or
executed. Reset **MUST** preserve a committed flash image and its CRC while
clearing volatile loader and running state.

After successful Start BPF transitions an image to running, Load BPF **MUST**
return `STATUS_BAD_STATE` until reset.

### REQ-OPT-BPF-007: Image CRC query

`CMD_QUERY_BPF_CRC` **MUST** require zero flags and an empty payload. If a
flash operation is pending, it **MUST** return `STATUS_BUSY`. If no committed
valid image exists, it **MUST** return `STATUS_NO_PROGRAM`. If the last
operation failed, it **MUST** return `STATUS_FLASH_ERROR` or
`STATUS_BAD_CRC`, as applicable. Otherwise it **MUST** return `STATUS_OK`
with the committed four-byte CRC-32 in little-endian order.

The CRC query **MUST NOT** mutate flash, map backing storage, status storage,
or loader/running state. Responses **MUST** retain the request sequence.

### REQ-OPT-BPF-008: Sonde compatibility boundary

The loader **MUST** preserve Sonde interpreter prerequisites for later
execution: eight-byte instruction alignment and non-aliasing, live map-region
bounds and map relocations derived from the committed array-map descriptors. It
**MUST NOT**
implement Sonde CBOR encoding, verification, map initial data, read-only maps,
or change the existing execution boundary in this change. The approved map
helper registration is limited to the requirements below.

### REQ-OPT-BPF-009: Loader statuses

The shared protocol **MUST** define these additional response statuses:

| Status | Value |
| --- | ---: |
| `STATUS_BUSY` | `0x05` |
| `STATUS_BAD_STATE` | `0x06` |
| `STATUS_BAD_CRC` | `0x07` |
| `STATUS_FLASH_ERROR` | `0x08` |
| `STATUS_NO_PROGRAM` | `0x09` |

### REQ-OPT-BPF-011: Runtime array-map creation

Each valid image descriptor **MUST** create one runtime
`BPF_MAP_TYPE_ARRAY` map before the image becomes executable. The runtime map
**MUST** retain its declared key size, value size, maximum entry count, and
fixed backing range. Only four-byte keys are supported.

### REQ-OPT-BPF-012: Atomic map creation

Map creation **MUST** occur only after the complete image, descriptors, bounds,
and CRC are validated. Failed or incomplete loads **MUST NOT** expose
partially created maps.

### REQ-OPT-BPF-013: Array-map helper registration

Start execution **MUST** register the pinned Sonde-compatible helper
descriptors:

| Helper | Sonde ID |
| --- | ---: |
| `bpf_map_lookup_elem` | `10` |
| `bpf_map_update_elem` | `11` |

Helpers **MUST** be available only during BPF execution and **MUST NOT**
allocate heap memory or mutate flash or unrelated protocol state.

### REQ-OPT-BPF-014: Array-map lookup

`bpf_map_lookup_elem` **MUST** validate the map reference and key pointer. The
key **MUST** be exactly four bytes and identify an entry in
`[0, max_entries)`. It **MUST** return a pointer to the entry's value, or null
for an out-of-range key. Returned pointers **MUST** remain within Sonde's
validated map-value region.

### REQ-OPT-BPF-015: Array-map update

`bpf_map_update_elem` **MUST** validate the map reference, key pointer, and
value pointer. It **MUST** require the exact declared key and value sizes and
an in-range array index.

The helper **MUST** copy the value into the selected entry and return zero on
success. Invalid maps, pointers, sizes, or indices **MUST** return a
deterministic negative failure value without escaping the interpreter safety
boundary.

### REQ-OPT-BPF-016: Map state and reset

Map contents **MUST** remain volatile and fixed-capacity. Maps **MUST** be
zero-initialized when runtime backing is initialized and after reset,
preserving the existing reset semantics.

Map helpers **MUST NOT** alter flash image bytes, image CRC, loader state, or
protocol status storage.

### REQ-OPT-BPF-017: Deterministic helper safety

Map helpers **MUST** be bounded, allocation-free, and compatible with Sonde
pointer-tagging and region validation. Existing interpreter failures **MUST**
retain their current `STATUS_BAD_COMMAND` mapping.

### REQ-OPT-BPF-018: One-shot timer scheduling helper

The firmware **MUST** register an OptiBridge-local Sonde-compatible helper with
ID `12` for one-shot timer scheduling. The helper **MUST** accept a bounded
`u32` delay in milliseconds and an opaque `u64` cookie. It **MUST** be
available only during BPF execution, **MUST NOT** allocate heap memory, and
**MUST NOT** invoke BPF recursively.

### REQ-OPT-BPF-019: Timer cancellation helper

The firmware **MUST** register an OptiBridge-local Sonde-compatible helper with
ID `13` for timer cancellation. The helper **MUST** accept no arguments, cancel
and clear the pending timer and its event state, and return zero. Calling it
when no timer is pending **MUST** also return zero.

### REQ-OPT-TMR-001: Timer replacement and one-shot behavior

An accepted schedule request **MUST** replace and cancel the existing pending
timer. Each accepted request **MUST** produce at most one timer event and
**MUST NOT** repeat implicitly. Periodic execution **MAY** be achieved only by
explicitly scheduling the next one-shot timer from BPF.

### REQ-OPT-TMR-002: Timer delay validation

The firmware **MUST** accept delays from `0` through `u32::MAX` milliseconds,
with zero meaning dispatch at the next available scheduler opportunity. A
delay outside that representable range is invalid by construction. Timer
backend failures **MUST** return a deterministic negative helper failure and
**MUST** preserve the currently pending timer.

### REQ-OPT-TMR-003: Generic event context

Timer expiry **MUST** invoke BPF with a fixed 32-byte context encoded as:

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 4 | event kind, `1` for timer |
| `4` | 4 | context version, `1` |
| `8` | 8 | exact `u64` cookie |
| `16` | 16 | source-specific payload, zero for timer |

All multi-byte fields **MUST** be little-endian. The context representation
**MUST** reserve the payload region for future DMA-completion and
GPIO-interrupt events without changing the BPF invocation ABI.

### REQ-OPT-TMR-004: Deferred timer execution

Timer expiry **MUST NOT** execute BPF from an interrupt handler, I2C callback,
or critical section. Expiry **MUST** signal bounded work to a firmware task or
equivalent runtime context, which then invokes BPF.

### REQ-OPT-TMR-005: Non-reentrant event execution

The firmware **MUST NOT** execute two BPF invocations concurrently or re-enter
the interpreter while an invocation is active. If a timer expires while BPF is
active, the firmware **MUST** retain one ready event and dispatch it only after
the active invocation completes. The ready event **MUST** be discarded if its
generation was canceled, replaced, or reset. A timer scheduled by the active
invocation applies to a future event and **MUST NOT** replace the invocation
already in progress.

### REQ-OPT-TMR-006: Reset and image lifecycle

Reset **MUST** cancel the pending timer and clear event-delivery state while
preserving the committed image and CRC. A stale timer or wakeup from before
reset **MUST NOT** invoke BPF afterward. Scheduling **MUST** require a
validated executable image.

### REQ-OPT-TMR-007: Cookie integrity

The `u64` cookie **MUST** pass from schedule request to BPF event context
without truncation or reinterpretation.

### REQ-OPT-TMR-008: Fixed timer resources

Timer state, event context, and wakeup signaling **MUST** use fixed-capacity,
allocation-free storage for one pending timer. Resource exhaustion or an
unavailable timer **MUST** return a deterministic helper failure without
overwriting unrelated state.

### REQ-OPT-TMR-009: Cancellation and stale wakeups

Cancellation **MUST** prevent BPF invocation even if a timer wakeup has
already been issued but not yet dispatched. Replacement scheduling and reset
**MUST** apply the same stale-wakeup suppression.

The implementation **MUST** use a generation or equivalent identity so that
only the current timer event can be dispatched. Cancellation **MUST NOT**
interrupt an invocation already in progress, but **MUST** clear any pending or
ready event that has not started.

### REQ-OPT-EVT-001: Future event-source compatibility

The event-delivery abstraction **MUST** represent the source and payload of an
event generically. This change **MUST** implement timer events only; GPIO,
DMA, optical, and other event sources **MUST** remain unimplemented.

### REQ-OPT-EVT-002: Existing Start compatibility

Synchronous `CMD_START_BPF` **MUST** retain its existing single-invocation
behavior, empty initial context, response statuses, and running-state
semantics. Timer helpers **MUST NOT** add a timer command or alter existing
map-helper behavior.

### REQ-OPT-MAP-READ-001: Read BPF map command

`CMD_READ_BPF_MAP` **MUST** require zero flags and exactly four payload bytes:
`map_id` (`u8`), `byte_offset` (`u16` little-endian), and `byte_length`
(`u8`). `byte_length` **MUST** be 1 through 16.

For a valid request, the firmware **MUST** return `STATUS_OK`, retain the
request sequence, and return exactly `byte_length` raw bytes from the selected
map's backing store at the half-open map-relative range
`[byte_offset, byte_offset + byte_length)`.

### REQ-OPT-MAP-READ-002: Map backing layout

After validating a committed image, firmware **MUST** derive every accepted
map's backing offset and byte length from descriptors in map-index order.
Map backing **MUST** remain within the fixed 1,024-byte allocation and
**MUST** be zero-initialized at boot. This change **MUST NOT** make map
contents persistent across reset.

### REQ-OPT-MAP-READ-003: Read errors and synchronization

`CMD_READ_BPF_MAP` with nonzero flags **MUST** return `STATUS_BAD_FLAGS`.
Malformed payload shape, zero or over-16 length, and a range that overflows or
falls outside the selected map **MUST** return `STATUS_BAD_LENGTH`. An
unknown `map_id` **MUST** return `STATUS_BAD_COMMAND`. When no valid committed
image exists, it **MUST** return `STATUS_NO_PROGRAM`.

Map reads **MUST NOT** mutate map backing, flash, loader state, or status
storage. They **MUST** hold synchronization only for the bounded copy of up
to 16 requested bytes.

### REQ-OPT-MAP-WRITE-001: Write BPF map command

`CMD_WRITE_BPF_MAP` **MUST** require zero flags and a payload of three through
16 bytes: `map_location` (`u16` little-endian) followed by one through 14 raw
replacement bytes. The replacement length is inferred from the payload length.
`map_location` bits `0..9` encode the map-relative byte offset, bits `10..12`
encode the map ID, and bits `13..15` are reserved and **MUST** be zero.

For a valid request, the firmware **MUST** replace exactly the selected map's
backing bytes at the half-open map-relative range
`[byte_offset, byte_offset + replacement_length)`, return status-only
`STATUS_OK`, and retain the request sequence.

### REQ-OPT-MAP-WRITE-002: Write map backing state

Map writes **MUST** use the descriptor-derived backing offsets and lengths
defined by REQ-OPT-MAP-READ-002. They **MUST NOT** modify backing outside the
selected range, flash, loader state, or status storage. Map backing remains
volatile and zero-initialized at boot; writes **MUST NOT** persist across reset.

### REQ-OPT-MAP-WRITE-003: Write errors and synchronization

`CMD_WRITE_BPF_MAP` with nonzero flags **MUST** return `STATUS_BAD_FLAGS`.
Payloads shorter than three bytes, nonzero reserved `map_location` bits, ranges
that fall outside the selected map, and a missing replacement byte **MUST**
return `STATUS_BAD_LENGTH`. A map ID that does not identify a map in the valid
committed image **MUST** return `STATUS_BAD_COMMAND`. When no valid committed
image exists, it **MUST** return `STATUS_NO_PROGRAM`. Rejected writes **MUST
NOT** mutate map backing.

Map writes **MUST** hold synchronization only for the bounded copy of up to 14
replacement bytes and response snapshot.

### REQ-OPT-VAL-021: End-to-end map-helper smoke test

The repository **MUST** provide a repeatable host-side smoke test that uses the
USB CDC/I2C bridge and target address `0x42` to exercise BPF image loading,
array-map reads and writes, one synchronous BPF invocation, and post-execution
map readback on hardware.

The test **MUST** fail explicitly on transport errors, malformed or busy
responses, unexpected statuses, short responses, sequence mismatches, or
value mismatches.

### REQ-OPT-VAL-022: Smoke-test BPF image

The smoke-test image **MUST** contain one four-byte-key,
four-byte-value `BPF_MAP_TYPE_ARRAY` map and a deterministic entry at index
zero. Its BPF program **MUST** call `bpf_map_lookup_elem`, verify that the
entry contains the expected value `41`, increment the value to `42`, and
persist the incremented value with `bpf_map_update_elem`.

The program **MUST** return zero only after lookup, comparison, increment, and
update succeed. Lookup failure, an unexpected value, or update failure **MUST**
produce a nonzero return value.

### REQ-OPT-VAL-023: Smoke-test transaction sequence

The smoke test **MUST** reset the target, wait the documented reset interval,
load and commit the smoke-test image, and wait for deferred load operations to
complete. It **MUST** then perform these operations in order:

1. Read map entry zero and verify four zero bytes.
2. Write the little-endian value `41` to map entry zero.
3. Issue exactly one valid `CMD_START_BPF` request and verify `STATUS_OK`.
4. Read map entry zero and verify the little-endian value `42`.

The test **MUST NOT** rely on a pre-existing image or map contents.

### REQ-OPT-VAL-024: Existing map-helper smoke-test scope

The existing synchronous map-helper smoke test **MUST NOT** introduce
recursive or timer-triggered execution, new map types, map deletion semantics,
or production protocol behavior outside the already specified load, map, and
Start command surfaces. The separate timer smoke test is governed by
REQ-OPT-VAL-025 through REQ-OPT-VAL-031.

### REQ-OPT-VAL-025: Timer smoke-test image

The timer hardware smoke test **MUST** load a valid BPF image containing one
4-byte array map and using map helpers `10` and `11`. The image **MUST** update
the map value once per invocation and schedule its next invocation with helper
`12`, a delay of `1000` milliseconds, and a deterministic cookie.

### REQ-OPT-VAL-026: Single host start

The timer smoke test **MUST** invoke `CMD_START_BPF` exactly once. Every later
BPF invocation **MUST** be caused by explicit one-shot timer rescheduling from
the BPF program.

### REQ-OPT-VAL-027: Timer progress observation

The host **MUST** periodically read the map and verify monotonic increments
across multiple timer periods without requiring exact wall-clock dispatch
times. The test **MUST** observe at least three increments during its default
observation window.

### REQ-OPT-VAL-028: Busy-read tolerance

The timer smoke test **MUST** retry map reads that return `STATUS_BUSY` during
BPF execution. Transient busy responses **MUST NOT** be treated as timer or
transport failures.

### REQ-OPT-VAL-029: Timer smoke-test failure handling

The test **MUST** fail on missed increments, unexpected decreases, malformed
responses, transport errors, or unexpected BPF/helper failures. It **MUST**
report the observed value and expected progress when failing.

### REQ-OPT-VAL-030: Timer smoke-test isolation

The timer smoke test **MUST** reset the target before and after execution so
that pending timer state does not leak into later tests.

### REQ-OPT-VAL-031: Existing smoke-test preservation

The existing synchronous map-helper smoke test **MUST** retain its `0 -> 41 ->
42` behavior and remain independently runnable.
