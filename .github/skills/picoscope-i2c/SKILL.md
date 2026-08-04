# PicoScope I2C Capture

Use this skill when debugging I2C traffic on the OptiBridge CH32V203G6U6
firmware with the connected PicoScope 2204A.

## Hardware setup

Verify the probes before capturing:

| PicoScope input | Board signal |
| --- | --- |
| Channel A | SDA / PB7 |
| Channel B | SCL / PB6 |
| Ground clip | Shared ground |

Use DC coupling and the 5 V range. I2C is 3.3 V logic; use 1.65 V as the
digital decode threshold.

## Required SDK API

On this Windows host, the 2204A is exposed through the legacy API:

```text
C:\Program Files\Pico Technology\SDK\lib\ps2000.dll
```

Do not use `ps2000a.dll`: it returns `PICO_NOT_FOUND` for this device.

`ps2000_open_unit` takes no arguments and returns a handle directly. A
positive return is success:

```python
api = ctypes.WinDLL(
    r"C:\Program Files\Pico Technology\SDK\lib\ps2000.dll"
)
api.ps2000_open_unit.argtypes = []
api.ps2000_open_unit.restype = ctypes.c_int16
handle = api.ps2000_open_unit()
if handle <= 0:
    raise RuntimeError(f"ps2000_open_unit failed: {handle}")
```

All legacy setup calls, including `ps2000_set_channel`, return a positive
value on success. Do not apply `ps2000a` status-code conventions to these
calls.

## Capture procedure

1. Open the scope with `ps2000_open_unit`.
2. Configure channel A and B with `ps2000_set_channel(handle, channel, 1, 1, 8)`.
   The values select enabled, DC coupling, and the 5 V range.
3. Query an available timebase with `ps2000_get_timebase`. With both channels
   enabled, use no more than the returned maximum sample count. A working
   configuration is timebase `8`, 3,500 samples, and 2.56 us/sample.
4. Configure `ps2000_set_trigger` on channel A, threshold
   `32767 * 1.65 / 5`, falling edge, no auto-trigger. This catches the I2C
   START condition.
5. Arm with `ps2000_run_block` before sending any bridge frame.
6. Open COM9 at 115200 baud, assert DTR and RTS, then send exactly one bridge
   request. Do not rely on the one-shot `READY` diagnostic banner.
7. Poll `ps2000_ready`, retrieve samples with `ps2000_get_values`, and save
   channel A as SDA and channel B as SCL in CSV form.
8. Decode a START as SDA falling while SCL is high. Decode each byte on SCL
   rising edges; the ninth bit is ACK when low and NACK when high.
9. Always close COM9 after the transaction. The bridge resets on DTR drop;
   wait for the USB CDC device to re-enumerate before opening the next session.

## Bridge frames

Frame format:

```text
A5 command payload-length sequence flags payload...
```

I2C write payload is `[address, bytes...]`; I2C read payload is
`[address, length]`.

For an SHT40 at `0x44`, write the high-precision measurement command:

```text
A5 10 02 01 00 44 FD
```

Then, after at least 20 ms, request six measurement bytes:

```text
A5 11 02 02 00 44 06
```

A successful SHT40 transaction returns response frames beginning:

```text
A5 00 00 01 00
A5 00 06 02 00
```

For the OptiBridge alive transaction, write `A5 01 00 01 00` to address
`0x42`, then read 10 bytes. A correct response payload is `alive`.

## Interpretation

- No START edge: the bridge did not reach I2C; verify the USB frame response
  with an invalid command before investigating the bus.
- Address byte followed by NACK: check target address, wiring, pull-ups, and
  target power/firmware.
- SDA or SCL permanently low: investigate the electrical bus or reset the
  attached target.
- Valid write but no later read data: inspect target-side state and the
  required command-to-read delay.

The known-good control firmware is
`embassy-i2c-sht40-smoke`; it configures PB6/PB7 as alternate-function
open-drain, initializes I2C1 master mode, and validates both blocking and
asynchronous SHT40 transactions.
