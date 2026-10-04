#!/usr/bin/env python3
"""
Staged, observable homing for the Dorna 1 arm via the v1 dorna Python API.

Usage:  python -u home_arm.py [joints...]
        python -u home_arm.py j0
        python -u home_arm.py j1 j2 j3 j4

Joints are homed in the order given, in a SINGLE home() call so the library's
j3/j4 wrist coupling works correctly. Position + homed status are printed
before and after. Always terminates cleanly so the serial port is released.

Safety: run with the arm clear and an e-stop in reach. Homing drives each axis
to its factory limit switch; the wrist (j3/j4) uses a probe routine on di4/di5.
"""
import sys, time
from dorna import Dorna

PORT = "/dev/ttyACM0"
joints = sys.argv[1:] or ["j0"]

r = Dorna()
try:
    print("[connect]", PORT, flush=True)
    dev = r.connect(PORT)
    print("[connected]", dev, flush=True)
    print("[before] homed:", r.homed(), flush=True)
    print("[before] joint:", r.position("joint"), flush=True)

    print(f"[homing] sequence: {joints}", flush=True)
    t0 = time.time()
    res = r.home(joints)
    dt = time.time() - t0
    print(f"[homing] home({joints}) -> {res}   ({dt:.1f}s)", flush=True)

    print("[after] homed:", r.homed(), flush=True)
    print("[after] joint:", r.position("joint"), flush=True)
    print("[after] xyz:  ", r.position("xyz"), flush=True)
finally:
    print("[terminate]", flush=True)
    r.terminate()
    print("DONE", flush=True)
