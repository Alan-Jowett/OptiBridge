# OptiBridge Firmware Design

## Components

| Component | Responsibility |
| --- | --- |
| Startup | Runs the Sonde size probe, initializes status storage to `ready`, and initializes clock, time, GPIO, and I2C slave resources. |
| Pin configuration | Selects I2C1 default routing and configures PB6/PB7 as released alternate-function open-drain pins. |
| I2C slave loop | Receives one I2C packet, packet-scoped parses it, dispatches at most one request, and writes a response. |
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
9. Enter the receive loop.

USB is not otherwise used by this firmware; its clock configuration is current
startup behavior.

## I2C receive state machine

```text
startup
  -> read I2C packet
  -> receive error: reset parser, read I2C packet
  -> reset parser
  -> parse packet bytes
  -> no complete frame / parser error: read I2C packet
  -> dispatch last complete frame
  -> write response (ignore result)
  -> read I2C packet
```

The parser resets before packet parsing. Each complete frame replaces the
previous candidate; therefore the final complete frame in one packet is the
only one dispatched. A parser error resets the parser and stops processing the
remainder of that packet.

## Action dispatch

The shared protocol keeps the existing 21-byte request and response buffers.
It validates flags before command-specific payload length. The action table is:

| Command | Valid request | Response | Runtime effect |
| --- | --- | --- | --- |
| Reset (`0x01`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Load BPF (`0x02`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Start BPF (`0x03`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Read BPF map (`0x04`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Write BPF map (`0x05`) | Empty | `STATUS_NOT_IMPLEMENTED` | None |
| Read Status (`0x06`) | Empty | `STATUS_OK` + newest message | Pop newest message |

`STATUS_NOT_IMPLEMENTED` is `0x04`. Unknown commands retain
`STATUS_BAD_COMMAND`. All action errors are status-only responses.

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
probe mismatch reach this handler. Receive errors clear parser state; response
write errors are discarded. No timeout, retry, I2C bus recovery, actual reset,
or other recovery mechanism is implemented.

## Resource model

All buffers are statically bounded or stack allocated. The firmware uses
`no_std`, performs no heap allocation, and obtains the interpreter solely from
the pinned external `sonde-bpf` dependency.

## Traceability

| Design element | Requirements |
| --- | --- |
| Platform and startup | REQ-OPT-FW-001, REQ-OPT-FW-002, REQ-OPT-FW-007 |
| Packet parser state machine | REQ-OPT-FW-003, REQ-OPT-FW-004, REQ-OPT-FW-006 |
| Action dispatch | REQ-OPT-FW-005, REQ-OPT-ACT-001, REQ-OPT-ACT-003, REQ-OPT-ACT-004, REQ-OPT-ACT-005, REQ-OPT-ACT-006 |
| Status storage | REQ-OPT-ACT-002, REQ-OPT-ACT-003, REQ-OPT-ACT-007 |
| Resource model | REQ-OPT-FW-008 |
