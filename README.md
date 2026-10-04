# dorna-arm-core

Control stack for the Dorna 1 robot arm (g2core on Arduino Due → 5 external step/dir drivers), hosted on the `ultra-arm` Raspberry Pi 3B (Tailscale 100.115.76.40). Snapshot pulled from the Pi on 2026-10-03; the Pi had no git, so this repo is the only off-device copy.

Long-term goal: prototype platform for the Mold Kitchen pouring station (toolhead pickup, vision-guided pours of metered material into mold cavities).

## Layout
- `arm-core/` — Rust daemon (intern, June 2026) that owns the controller serial port and serves a line-delimited JSON API on `/tmp/arm-core.sock`; `gateway/` is the HTTP edge + static console on :2600. See `arm-core/README.md`.
  - 2026-10-03: added a wire log (`src/transport.rs`, `WireLog`) → `/home/intern/arm-core/logs/serial.log`, every TX/RX line, idle polls deduped, 10 MB rotation.
- `arm-web-dist/` — the web debug console. 2026-10-03 additions: link-health panel (reads `net.json`), base (X) disabled via `DISABLED_AXES`, home buttons removed, joystick driving via browser Gamepad API (trigger = deadman, thumb = STOP, streaming pacing, speed slider, key-mapping learn mode).
- `deploy/` — systemd units (`arm-core`, `arm-gateway`, `arm-netwatch`) and the `arm-netwatch` collector script (writes `net.json`: wifi/ethernet/undervoltage/events).
- `home_arm.py`, `plating/` — intern's early scripts using the official `dorna` Python library. Avoid for motion: that library sends large init moves on connect.

## Build / deploy (on the Pi)
```
cd ~/arm-core && cargo build --release
sudo systemctl restart arm-core arm-gateway   # NOTE: restarting arm-core REBOOTS the controller (opening the port resets the Due) — support the arm first
```

## Known state (2026-10-03)
- Base (X / driver #1): loose B− coil wire → reversals, noise; disabled until reseated. Also `{"1pm":0}` (resets on controller reboot).
- Home sensors unverified; an X home search on 2026-10-03 drove 800°+ without the sensor tripping. Do not home an axis until its input is seen flipping.
- Pi PSU undervolts under load (wifi drops); use ethernet + a 2.5 A+ supply / powered USB hub.
- Console axis letters are joints (X base, Y shoulder, Z elbow, A/B differential wrist motors, C unused); units are joint degrees.
