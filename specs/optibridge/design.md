# OptiBridge Firmware Design

## Components

| Component | Responsibility |
| --- | --- |
| Startup | Runs the Sonde size probe and initializes clock, time, GPIO, and I2C slave resources. |
| Pin configuration | Selects I2C1 default routing and configures PB6/PB7 as released alternate-function open-drain pins. |
| I2C slave loop | Receives one I2C packet, packet-scoped parses it, dispatches at most one request, and writes a response. |
| Shared protocol | Supplies bounded frame parsing and `alive` command/status dispatch. |
| Sonde probe | Retains the external no-std interpreter and validates a fixed `mov r0, 42; exit` program. |

## Startup sequence

1. Execute the Sonde BPF size probe.
2. Panic if the interpreter result is not `Ok(42)`.
3. Configure the 48 MHz USB FS device clock and Embassy time runtime.
4. Enable and release-reset GPIOB.
5. Select I2C1 default routing, release PB6/PB7 high, and configure both pins
   as 50 MHz alternate-function open-drain.
6. Enable and release-reset I2C1 slave resources.
7. Initialize I2C1 slave mode and set own address `0x42`.
8. Enter the receive loop.

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

## Protocol dispatch

`dispatch` uses fixed 21-byte request and response buffers. It accepts only:

```text
A5 01 00 <sequence> 00
```

and returns:

```text
A5 00 05 <sequence> 00 61 6C 69 76 65
```

All other complete requests receive the appropriate status-only response from
the shared protocol dispatcher.

## Failure behavior

The panic handler spins indefinitely. Startup `unwrap` failures and Sonde
probe mismatch reach this handler. Receive errors clear parser state; response
write errors are discarded. No timeout, retry, I2C bus recovery, reset
command, or other recovery mechanism is implemented.

## Resource model

All buffers are statically bounded or stack allocated. The firmware uses
`no_std`, performs no heap allocation, and obtains the interpreter solely from
the pinned external `sonde-bpf` dependency.

## Traceability

| Design element | Requirements |
| --- | --- |
| Platform and startup | REQ-OPT-FW-001, REQ-OPT-FW-002 |
| Packet parser state machine | REQ-OPT-FW-003, REQ-OPT-FW-004, REQ-OPT-FW-008 |
| Frame dispatch | REQ-OPT-FW-005, REQ-OPT-FW-006, REQ-OPT-FW-007 |
| Sonde probe | REQ-OPT-FW-009 |
| Resource model | REQ-OPT-FW-010 |
