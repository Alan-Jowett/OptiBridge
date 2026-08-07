# OptiBridge Firmware Design

## Components

| Component | Responsibility |
| --- | --- |
| Startup | Runs the Sonde size probe, initializes status storage to `ready`, validates any committed BPF flash-image header, and initializes clock, time, GPIO, and I2C slave resources. |
| Pin configuration | Selects I2C1 default routing and configures PB6/PB7 as released alternate-function open-drain pins. |
| I2C ISR dispatch | Receives one bounded packet, dispatches at most one request, and replaces the single response slot. |
| Shared protocol | Defines action commands/statuses and dispatches bounded action requests. |
| Status storage | Holds one newest status message, initially `ready`; reads consume it. |
| Sonde probe | Retains the external no-std interpreter and validates a fixed `mov r0, 42; exit` program. |
| BPF image loader | Streams a compact native image into two reserved flash pages without a page-sized RAM staging buffer. |
| BPF CRC query | Reports the CRC-32 of a validated committed image without executing it. |
| BPF runtime maps | Instantiates fixed-size array maps from validated image descriptors in the volatile 1,024-byte backing store. |
| BPF map helpers | Registers Sonde helper IDs 10 and 11 for array-map lookup and update during Start execution. |

## Startup sequence

1. Execute the Sonde BPF size probe.
2. Panic if the interpreter result is not `Ok(42)`.
3. Validate the reserved-image header and its CRC; retain only a valid committed image.
4. Initialize the newest-status storage to ASCII `ready`.
5. Configure the 48 MHz USB FS device clock and Embassy time runtime.
6. Enable and release-reset GPIOB.
7. Select I2C1 default routing, release PB6/PB7 high, and configure both pins
   as 50 MHz alternate-function open-drain.
8. Enable and release-reset I2C1 slave resources.
9. Initialize I2C1 slave mode and set own address `0x42`.
10. Register the I2C RX-packet ISR dispatcher with its static 21-byte receive
   buffer.
11. Enter the idle loop while I2C interrupts process packets and deferred
    flash operations.

USB is not otherwise used by this firmware; its clock configuration is current
startup behavior.

## I2C ISR state machine

```text
startup
  -> enable RX packet ISR dispatch
  -> receive completed packet
  -> packet is truncated: queue empty packet
  -> packet-local parse
  -> incomplete or malformed: queue empty packet
  -> dispatch final complete frame
  -> dispatch Reset: set reset pending, return from ISR
  -> main loop: reset MCU
  -> queue bounded response
  -> remain armed for next master write
```

`dispatch_packet` is a shared, target-independent helper. It creates
packet-local parser state and returns `PacketOutcome::Response(length)` or
`PacketOutcome::Empty`. A truncated packet returns `Empty` before parsing.
Each complete frame replaces the previous candidate; therefore only the final
complete frame in a non-truncated packet is dispatched. Packets over the
21-byte receive bound are truncated and do not dispatch any captured prefix.

The generated callback is a bare function pointer. Callback-visible status and
response state is stored in a `critical_section::Mutex<RefCell<...>>` static;
the callback takes a bounded response snapshot, releases that state, then
queues exactly one packet. `Response` queues its frame and `Empty` queues
`&[]`, actively replacing an unread response.

The generated response mechanism is one 32-byte overwrite slot, not a FIFO.
Masters must use write/read ordering. A later valid write replaces an unread
response. An empty slot may yield zero filler on a later read; those bytes are
outside the shared frame protocol.

## Action dispatch

The shared protocol keeps the existing 21-byte request and response buffers.
It validates flags before command-specific payload length. The action table is:

| Command | Valid request | Response | Runtime effect |
| --- | --- | --- | --- |
| Reset (`0x01`) | Empty | No response | Immediate generated-HAL system reset |
| Load BPF (`0x02`) | Begin, Data, or Finalize | `STATUS_OK` on acceptance | Deferred flash-image operation |
| Start BPF (`0x03`) | Empty, zero flags, committed image | Status-only `STATUS_OK`, `STATUS_NO_PROGRAM`, or `STATUS_BAD_COMMAND` | Invoke image exactly once |
| Read BPF map (`0x04`) | Map ID + byte range | `STATUS_OK` + raw bytes | Bounded map snapshot |
| Write BPF map (`0x05`) | Map ID + byte range + raw bytes | Status-only `STATUS_OK` | Bounded map update |
| Read Status (`0x06`) | Empty | `STATUS_OK` + newest message | Pop newest message |
| Query BPF CRC (`0x07`) | Empty | CRC, busy, no-program, or loader error | None |

`STATUS_NOT_IMPLEMENTED` is `0x04`. `STATUS_BUSY`, `STATUS_BAD_STATE`,
`STATUS_BAD_CRC`, `STATUS_FLASH_ERROR`, and `STATUS_NO_PROGRAM` are `0x05`
through `0x09`. Unknown commands retain `STATUS_BAD_COMMAND`. A valid Reset is
fire-and-forget and does not queue a response. The master waits at least one
second before its next I2C request. Reset validation errors and all other
action errors are status-only responses.

Start BPF executes in the bounded I2C request callback before its status-only
response is queued. It supplies an empty context, registers the Sonde array-map
helpers (IDs 10 and 11), passes the validated map backing ranges, and invokes
the image once. Return value zero enters the running state and returns `STATUS_OK`;
nonzero returns or interpreter errors return `STATUS_BAD_COMMAND` without
entering that state. A running image rejects Start and Load BPF with
`STATUS_BAD_STATE`, while CRC and map operations retain their read-only or
bounded semantics.

## BPF image and flash state machine

The linker reserves flash offsets `0x6000..=0x7fff` as two 4,096-byte image
pages; executable firmware links below `0x6000`. Image offset zero begins with
the 16-byte `OBPF` v1 header:

| Image bytes | Field |
| --- | --- |
| `0..4` | ASCII `OBPF` |
| `4` | Format version `1` |
| `5` | Map count |
| `6..8` | Little-endian bytecode length |
| `8..12` | Little-endian CRC-32/ISO-HDLC |
| `12..14` | `0xFFFF` while incomplete; `0x0000` when committed |
| `14..16` | Erased (`0xFFFF`) |

The image payload is bytecode followed by 16-byte little-endian map
descriptors. The CRC-32/ISO-HDLC covers payload bytes only, in that exact
order. Sonde `BPF_MAP_TYPE_ARRAY` is encoded as `map_type = 1`. A committed
descriptor creates one fixed-size runtime array map; map creation is atomic
with image validation and never exposes a partially loaded map.

```text
no-image
  -> Begin accepted -> erase/program pending -> receiving
  -> Data accepted -> program pending -> receiving
  -> Finalize accepted -> validate/commit pending -> committed
  -> flash or CRC failure -> failed
committed -> Begin accepted -> erase/program pending
receiving or failed -> Reset -> no-image
committed -> Reset -> committed-on-flash after boot validation
committed -> successful Start BPF -> running -> Reset -> committed
```

The callback only validates a frame, copies one Load BPF operation into a fixed
pending record, and acknowledges it. The main loop erases pages, programs
halfwords, advances the expected offset, updates the rolling CRC, validates
descriptors, and writes the commit marker last. The host serializes operations:
it waits for `CMD_QUERY_BPF_CRC` to stop returning `STATUS_BUSY` before issuing
the next Load BPF request. A busy Load BPF request was not accepted and may be
retried unchanged.

Begin declares bytecode length, map count, and the expected CRC. Data carries
an exact ascending image offset and an even 2-12 byte payload. Finalize has no
data. The maximum image is 7,680 bytes of bytecode plus at most eight
descriptors; it fits the two-page reservation with the header. The main loop
checks all map products and their aggregate 1,024-byte RAM allocation before
writing the commit marker.

## Map state

Validated descriptors are laid out consecutively in the fixed 1,024-byte map
backing store. A descriptor's backing length is `value_size * max_entries`;
its map ID is its zero-based descriptor index. Backing is zeroed at boot and
is volatile. Start registers Sonde helper ID 10
(`bpf_map_lookup_elem`) and helper ID 11 (`bpf_map_update_elem`).

Array lookup requires a four-byte key containing an in-range array index and
returns a tagged pointer to the selected value or null for an out-of-range
index. Array update requires the exact declared key and value sizes, copies
the value into the selected entry, and returns zero on success or a negative
failure value. Neither helper allocates, accesses flash, or changes loader
state. Array-map deletion is intentionally unsupported.

Read BPF map accepts `[map_id, byte_offset_le[0], byte_offset_le[1],
byte_length]`. It copies the requested one-through-16-byte map-relative range
into the shared response while holding the protocol state only for that copy.
No valid image returns `STATUS_NO_PROGRAM`; an unknown map returns
`STATUS_BAD_COMMAND`; malformed or out-of-range byte ranges return
`STATUS_BAD_LENGTH`.

Write BPF map accepts `[map_location_le[0], map_location_le[1],
replacement_bytes...]`. `map_location` packs the 10-bit map-relative byte
offset in bits `0..9` and the 3-bit map ID in bits `10..12`; bits `13..15` are
zero. Its one-through-14-byte replacement range is inferred from the payload
length. It synchronously replaces exactly that map-relative range while holding
shared protocol state only for the bounded copy and response snapshot. It
returns status-only `STATUS_OK`; no valid image, unknown map, and malformed or
out-of-range ranges return the same respective statuses as reads. Writes are
volatile: startup clears all backing before accepting requests.

At startup the main loop validates header magic/version/marker, bounds, map
descriptors, and a recomputed CRC before treating an image as committed. The
CRC query reports only this validated committed CRC. A partially programmed
page is therefore never executable or queryable as a program.

## End-to-end smoke test

The host-side smoke test runs through the USB CDC/I2C bridge at target address
`0x42`. It resets the target, waits at least one second, loads and commits a
single-map image, and waits for each deferred load operation to finish before
continuing.

The image contains one array map with a four-byte key, four-byte value, and one
entry. The BPF program uses key zero, looks up the entry through helper ID 10,
requires the little-endian value `41`, increments it to `42`, and updates the
entry through helper ID 11. The host sequence reads zeroed backing, writes
little-endian `41`, starts the image exactly once, and reads back little-endian
`42`.

The smoke test treats any transport failure, unexpected status, sequence
mismatch, short response, failed BPF execution, or map value mismatch as a
failure. It does not add runtime behavior or exercise recursive or
event-triggered execution.

## Status storage

Status storage is one fixed 16-byte message buffer plus its length. Startup
writes `ready`; no current action replaces it. Read Status encodes the stored
length and bytes, then clears the stored length. When the queue is empty, it
returns `STATUS_OK` with an empty payload.

This is intentionally a single-entry status queue. Future sources may replace
the newest message only after a separate specification defines their ordering,
capacity, and overflow behavior.

## Failure behavior

The panic handler spins indefinitely. Startup `unwrap` failures and Sonde
probe mismatch reach this handler. Every completed packet replaces the response
slot, including rejected packets that queue an empty response, except valid
Reset, which sets an atomic pending flag. The main loop invokes generated
`interrupt::system_reset()` after the I2C ISR returns. Reset reruns startup and
the Sonde probe; a probe failure leaves the target unavailable. Low-level bus
errors that abort before packet completion are an accepted deferred gap: they
do not enter the callback and can leave an unread generated response slot
unchanged. No timeout, retry, I2C bus recovery, or other recovery mechanism is
implemented.

## Resource model

All buffers are statically bounded or stack allocated. The firmware uses
`no_std`, performs no heap allocation, and obtains the interpreter solely from
the pinned external `sonde-bpf` dependency. The loader has no 4,096-byte RAM
page buffer: flash pages are erased once, then programmed in ascending
halfword-sized fragments. The 1,024-byte map backing store is the only new
dedicated runtime RAM reservation; final linking must leave at least 4,096
bytes of execution stack.

## Traceability

| Design element | Requirements |
| --- | --- |
| Platform and startup | REQ-OPT-FW-001, REQ-OPT-FW-002, REQ-OPT-FW-007 |
| I2C ISR state machine | REQ-OPT-FW-003, REQ-OPT-FW-004, REQ-OPT-FW-006, REQ-OPT-FW-009 to REQ-OPT-FW-013 |
| Action dispatch | REQ-OPT-FW-005, REQ-OPT-FW-014, REQ-OPT-ACT-001, REQ-OPT-ACT-003 to REQ-OPT-ACT-009, REQ-OPT-BPF-010, REQ-OPT-MAP-WRITE-001 to REQ-OPT-MAP-WRITE-003 |
| Status storage | REQ-OPT-ACT-002, REQ-OPT-ACT-003, REQ-OPT-ACT-007 |
| Resource model | REQ-OPT-FW-008 |
| BPF image, execution, and flash state machine | REQ-OPT-BPF-001 to REQ-OPT-BPF-017 |
| Map state and helpers | REQ-OPT-BPF-011 to REQ-OPT-BPF-017, REQ-OPT-MAP-READ-001 to REQ-OPT-MAP-READ-003, REQ-OPT-MAP-WRITE-001 to REQ-OPT-MAP-WRITE-003 |
| End-to-end smoke test | REQ-OPT-VAL-021 to REQ-OPT-VAL-024 |
