# OptiBridge Firmware Design

## Components

| Component | Responsibility |
| --- | --- |
| Startup | Runs the Sonde size probe, initializes status storage to `ready`, and initializes clock, time, GPIO, and I2C slave resources. |
| Pin configuration | Selects I2C1 default routing and configures PB6/PB7 as released alternate-function open-drain pins. |
| I2C ISR dispatch | Receives one bounded packet, dispatches at most one request, and replaces the single response slot. |
| Shared protocol | Defines action commands/statuses and dispatches bounded action requests. |
| Status storage | Holds one newest status message, initially `ready`; reads consume it. |
| Sonde probe | Retains the external no-std interpreter and validates a fixed `mov r0, 42; exit` program. |

## Startup sequence

1. Execute the Sonde BPF size probe.
2. Panic if the interpreter result is not `Ok(42)`.
3. Initialize the newest-status storage to ASCII `ready`.
4. Configure the 48 MHz USB FS device clock and Embassy time runtime.
5. Enable and release-reset GPIOB.
6. Select I2C1 default routing, release PB6/PB7 high, and configure both pins
   as 50 MHz alternate-function open-drain.
7. Enable and release-reset I2C1 slave resources.
8. Initialize I2C1 slave mode and set own address `0x42`.
9. Register the I2C RX-packet ISR dispatcher with its static 21-byte receive
   buffer.
10. Enter the idle loop while I2C interrupts process packets.

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
| Load BPF (`0x02`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Start BPF (`0x03`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Read BPF map (`0x04`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Write BPF map (`0x05`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Read Status (`0x06`) | Empty | `STATUS_OK` + newest message | Pop newest message |

`STATUS_NOT_IMPLEMENTED` is `0x04`. Unknown commands retain
`STATUS_BAD_COMMAND`. A valid Reset is fire-and-forget and does not queue a
response. The master waits at least one second before its next I2C request.
Reset validation errors and all other action errors are status-only responses.

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
the pinned external `sonde-bpf` dependency.

## Traceability

| Design element | Requirements |
| --- | --- |
| Platform and startup | REQ-OPT-FW-001, REQ-OPT-FW-002, REQ-OPT-FW-007 |
| I2C ISR state machine | REQ-OPT-FW-003, REQ-OPT-FW-004, REQ-OPT-FW-006, REQ-OPT-FW-009 to REQ-OPT-FW-013 |
| Action dispatch | REQ-OPT-FW-005, REQ-OPT-FW-014, REQ-OPT-ACT-001, REQ-OPT-ACT-003 to REQ-OPT-ACT-009 |
| Status storage | REQ-OPT-ACT-002, REQ-OPT-ACT-003, REQ-OPT-ACT-007 |
| Resource model | REQ-OPT-FW-008 |
