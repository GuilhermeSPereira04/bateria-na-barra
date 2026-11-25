# Zaopin receiver battery protocol

Receiver: `VID 3554 / PID F527` (product string "Z4D"), Compx-based. Official
software: "Zaopin Wireless Mouse" (Qt app + `HIDUsb.dll`), installed at
`D:\Zaopin Wireless Mouse`.

## How it was found (2026-11-24)

Static reading of the official driver instead of a USB capture (no extra
driver needed). The app (`Zaopin Wireless Mouse.exe`) imports
`UsbServer_ReadBatteryLevel` and `GetDeviceBatteryStatus` from `HIDUsb.dll`.

- `UsbServer_ReadBatteryLevel` / `CS_UsbFinder_GetBatteryLevel` build a 17-byte
  buffer: `[0]=0x08`, `[1]=0x04`, rest zero, `[16]=checksum(bytes 0..16)`.
  Byte 5 gets `|= 0x80` only in keyboard mode (`*_SetKeyboardMode`); the mouse
  uses mouse mode, so byte 5 stays 0.
- Checksum (`HIDUsb.dll` RVA `0x114c0`): `0x55 - sum(bytes)` (wrapping u8).
- Transport (RVA `0xdcd0`): overlapped `ReadFile` of 17 bytes posted, then
  `WriteFile` of the 17-byte request, `WaitForSingleObject` 100 ms, verify the
  reply checksum and that reply byte 1 equals the command; up to 3 tries.
- `CS_GetDeviceBatteryStatus` only unpacks the reply (`level, status, mV`); the
  app shows `level` as-is and treats `status > 0` as charging.

Collection: interface 1, usage page `0xFF02` — its descriptor declares report
`0x08` with 16 bytes in and 16 bytes out (see `zaopin-probe.txt`).

## Request / reply

```
request  08 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 49
reply    08 04 00 00 00 02 19 00 0E 3D 00 00 00 00 00 00 E3
               |        |  |  |  |                       '- checksum
               |        |  |  '--'- millivolts, big endian (0x0E3D = 3645)
               |        |  '- status: > 0 = charging
               |        '- battery percent (0x19 = 25)
               '- echoes the command
```

Observed: 25 % / 3637–3645 mV, not charging. The official app showed 23 % at
another moment; the percentage is computed by the mouse from its voltage and
moves a little with load.

Not yet observed: a reply while charging (expected status > 0) and the reply
while the mouse is asleep (the app then shows the held value, then `--%`).
