#!/usr/bin/env python3
"""
Read-only backup of the Dorna 1 g2core controller configuration.
Captures the full `$$` text dump (canonical, restorable form) plus per-group
JSON (motors 1-6, axes x..c, system, coordinate offsets). No motion commanded.
"""
import serial, time, sys

PORT = "/dev/ttyACM0"

def drain(ser, idle=1.5, cap=20):
    """Read until no new data for `idle` seconds, or `cap` seconds total."""
    buf = b""
    last = time.time()
    start = time.time()
    while time.time() - start < cap:
        d = ser.read(4096)
        if d:
            buf += d
            last = time.time()
        elif time.time() - last > idle:
            break
    return buf.decode(errors="replace")

ser = serial.Serial(PORT, 115200, timeout=0.2)
time.sleep(0.6)
ser.reset_input_buffer()

print("# Dorna 1 / g2core controller config backup")
print("# port:", PORT, "  captured (Pi local time):", time.strftime("%Y-%m-%d %H:%M:%S %Z"))

# provenance
for q in [b'{"fv":null}\n', b'{"fb":null}\n', b'{"hp":null}\n', b'{"id":null}\n']:
    ser.write(q); ser.flush(); time.sleep(0.25)
print("\n## provenance (JSON)\n" + drain(ser).strip())

# full text dump — the canonical g2core backup
ser.reset_input_buffer()
ser.write(b"$$\n"); ser.flush()
print("\n## full settings ($$ text dump)\n" + drain(ser, idle=2.0, cap=25).strip())

# structured JSON groups (re-sendable form)
print("\n## settings by group (JSON)")
groups = [b"1", b"2", b"3", b"4", b"5", b"6",
          b"x", b"y", b"z", b"a", b"b", b"c",
          b"sys", b"g54", b"g55", b"g28", b"g30"]
ser.reset_input_buffer()
for g in groups:
    ser.write(b'{"' + g + b'":null}\n'); ser.flush(); time.sleep(0.3)
print(drain(ser).strip())

ser.close()
print("\n# end of backup")
