// arm-core debug console: poll state + hall inputs, jog/home per axis, stop,
// raw g2core passthrough. Plain DOM + fetch, no framework. Talks to the
// arm-core HTTP+JSON API via ./api.js.
import * as api from "./api.js";
const AXES = ["X", "Y", "Z", "A", "B", "C"];
// g2core stat codes -> readable names.
const STAT = ["init", "ready", "alarm", "stop", "end", "run", "hold", "probe", "cycle", "homing"];
// Which hall input each axis homes against (J0=X uses in6, etc.).
const HOMING_INPUT = { 2: 1, 3: 1, 4: 1, 5: 1, 6: 1 };
function el(id) {
    return document.getElementById(id);
}
function dot(on) {
    return `<span class="dot ${on ? "on" : ""}"></span>`;
}
function fmt(v) {
    return v === undefined ? "—" : v.toFixed(3);
}
function setConn(ok, msg) {
    const c = el("conn");
    c.textContent = msg;
    c.className = "conn " + (ok ? "ok" : "bad");
}
// --- base URL field ---------------------------------------------------------
const baseInput = el("base");
baseInput.value = api.getBase();
baseInput.addEventListener("change", () => api.setBase(baseInput.value));
// --- jog config -------------------------------------------------------------
const step = () => parseFloat(el("step").value) || 0;
const feed = () => parseFloat(el("feed").value) || 500;
function makeButton(label, onClick) {
    const b = document.createElement("button");
    b.textContent = label;
    b.addEventListener("click", onClick);
    return b;
}
async function doJog(axis, delta) {
    try {
        await api.jog(axis, delta, feed());
    }
    catch (e) {
        alert(`jog ${axis} failed: ${e}`);
    }
}
async function doHome(axis) {
    if (axis === "X" && !confirm("J0 (X) homing is known-broken hardware. Home anyway?"))
        return;
    try {
        await api.home(axis);
    }
    catch (e) {
        alert(`home ${axis} failed: ${e}`);
    }
}
// Build per-axis jog/home rows.
const DISABLED_AXES = ["X"];
const axesDiv = el("axes");
for (const ax of AXES) {
    const row = document.createElement("div");
    row.className = "axisrow";
    const name = document.createElement("span");
    name.className = "axname";
    name.textContent = ax;
    // 2026-10-03: base (X) disabled pending coil-wire repair on driver #1; homing
    // disabled until each joint home sensor is verified (X home search crashed).
    if (DISABLED_AXES.includes(ax)) {
        const note = document.createElement("span");
        note.textContent = "disabled (driver #1 wiring)";
        row.append(name, note);
    } else {
        row.append(name, makeButton("-", () => doJog(ax, -step())), makeButton("+", () => doJog(ax, +step())));
    }
    axesDiv.append(row);
}
// --- stop -------------------------------------------------------------------
function doStop() {
    api.stop().catch((e) => alert(`stop failed: ${e}`));
}
el("stop").addEventListener("click", doStop);
// --- raw passthrough --------------------------------------------------------
el("rawform").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const line = el("rawline").value.trim();
    if (!line)
        return;
    try {
        const r = await api.raw(line);
        el("rawout").textContent = JSON.stringify(r.responses, null, 2);
    }
    catch (e) {
        el("rawout").textContent = String(e);
    }
});
// --- rendering --------------------------------------------------------------
function renderState(sr) {
    const pos = {
        X: sr.posx, Y: sr.posy, Z: sr.posz, A: sr.posa, B: sr.posb, C: sr.posc,
    };
    const homed = {
        X: sr.homx, Y: sr.homy, Z: sr.homz, A: sr.homa, B: sr.homb,
    };
    const statName = STAT[sr.stat ?? -1] ?? `stat ${sr.stat}`;
    el("summary").innerHTML =
        `<b>stat</b> ${statName} <b>vel</b> ${sr.vel ?? "—"} ` +
            `<b>all homed</b> ${sr.home ? "yes" : "no"} <b>queue</b> ${sr.qr ?? "—"}`;
    const body = el("state").tBodies[0];
    body.innerHTML = AXES.map((ax) => {
        const h = ax === "C" ? "" : dot(!!homed[ax]);
        return `<tr><td>${ax}</td><td>${fmt(pos[ax])}</td><td>${h}</td></tr>`;
    }).join("");
}
function renderInputs(inputs) {
    el("inputs").innerHTML = Object.keys(inputs)
        .map(Number)
        .sort((a, b) => a - b)
        .map((n) => {
        const cls = HOMING_INPUT[n] ? "in homing" : "in";
        return `<span class="${cls}">${dot(inputs[String(n)] === 1)} in${n}</span>`;
    })
        .join("");
}
// Latest controller status, shared with the joystick pacing logic.
let lastSr = null;
let lastSrAt = 0; // browser time (ms) of lastSr
function noteStatus(sr) {
    lastSr = sr;
    lastSrAt = Date.now();
}
// --- poll loop --------------------------------------------------------------
async function refresh() {
    try {
        const [sr, inputs] = await Promise.all([api.getState(), api.getInputs()]);
        renderState(sr);
        noteStatus(sr);
        renderInputs(inputs);
        setConn(true, "connected");
    }
    catch (e) {
        setConn(false, `offline: ${e}`);
    }
}
refresh();
setInterval(refresh, 1000);
// --- link health (Pi wifi/power, from /net.json written by arm-netwatch) -----
let lastNetOk = 0; // browser time (ms) of last successful /net.json fetch
function hms(sec) {
    const d = new Date(sec * 1000);
    return [d.getHours(), d.getMinutes(), d.getSeconds()].map((n) => String(n).padStart(2, "0")).join(":");
}
function esc(s) {
    return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
}
function renderBadge() {
    const b = el("lh-badge");
    if (!lastNetOk) {
        b.textContent = "LINK ?";
        b.className = "lh-badge";
        return;
    }
    const age = (Date.now() - lastNetOk) / 1000;
    if (age < 4) {
        b.textContent = "LINK OK";
        b.className = "lh-badge ok";
    }
    else {
        b.textContent = `PI UNREACHABLE \u2014 ${Math.floor(age)}s`;
        b.className = "lh-badge bad";
    }
}
function renderNet(n) {
    let sig = "\u2014", cls = "";
    if (n.link_up && typeof n.signal_dbm === "number") {
        const w = n.signal_dbm >= -60 ? "good" : n.signal_dbm >= -70 ? "fair" : "poor";
        sig = `<span class="lh-${w}">${n.signal_dbm} dBm (${w})</span>`;
    }
    el("lh-wifi").innerHTML =
        `<div><b>link</b> <span class="${n.link_up ? "lh-ok" : "lh-bad"}">${n.link_up ? "up" : "DOWN"}</span></div>` +
            `<div><b>ssid</b> ${esc(n.ssid ?? "\u2014")}</div>` +
            `<div><b>signal</b> ${sig}</div>` +
            `<div><b>bitrate</b> ${esc(n.tx_bitrate ?? "\u2014")}</div>` +
            `<div><b>ip</b> ${esc(n.ip ?? "\u2014")}</div>` +
            `<div><b>ethernet plugged</b> ${n.eth0_up ? "yes" : "no"}</div>`;
    el("lh-power").innerHTML =
        `<div>${n.undervoltage_now ? '<span class="lh-bad">UNDERVOLTAGE NOW</span>' : '<span class="lh-ok">Power OK</span>'}</div>` +
            (n.undervoltage_since_boot ? `<div class="lh-fair">undervoltage seen since boot</div>` : "") +
            `<div><b>wifi drops since boot</b> ${n.drops_since_boot}</div>` +
            `<div><b>undervoltage events since boot</b> ${n.undervoltage_count_since_boot}</div>` +
            `<div><b>uptime</b> ${Math.floor(n.uptime_s / 60)} min</div>`;
    el("lh-events").innerHTML = (n.events || [])
        .map((e) => `<li class="${esc(e.type)}"><span class="t">${hms(e.ts)}</span>${esc(e.type)}` +
        (e.detail ? ` <span class="t">${esc(e.detail)}</span>` : "") + `</li>`)
        .join("") || "<li class=\"t\">none</li>";
    const age = Math.max(0, Date.now() / 1000 - n.ts);
    el("lh-age").textContent = `Pi data age ~${Math.round(age)}s (browser clock vs Pi clock)`;
}
async function refreshNet() {
    const ctl = new AbortController();
    const to = setTimeout(() => ctl.abort(), 1500);
    try {
        const r = await fetch("/net.json", { cache: "no-store", signal: ctl.signal });
        if (!r.ok)
            throw new Error(String(r.status));
        const n = await r.json();
        lastNetOk = Date.now();
        renderNet(n);
    }
    catch (e) {
        // leave last render in place; the badge ages toward PI UNREACHABLE
    }
    finally {
        clearTimeout(to);
    }
    renderBadge();
}
refreshNet();
setInterval(refreshNet, 2000);
setInterval(renderBadge, 1000);
// --- joystick (browser Gamepad API; Thrustmaster T.Flight Stick X) -----------
// Joystick is plugged into the computer running the browser, not the Pi.
// Edit this table to remap. src: "axis" (analog, uses gamepad.axes[idx]) or
// "btnpair" (hat/D-pad as buttons: neg/pos button index). sign flips direction.
// max = degrees per jog at full deflection (analog) or fixed degrees per tick
// while held (btnpair).
const JOY_MAP = [
    { axis: "Y", src: "axis", idx: 1, sign: -1, max: 2 }, // stick fwd/back -> shoulder (fwd = axes[1] negative)
    { axis: "Z", src: "axis", idx: 0, sign: +1, max: 2 }, // stick left/right -> elbow
    { axis: "A", src: "btnpair", pos: 12, neg: 13, sign: +1, max: 5 }, // hat up/down (std buttons 12 up, 13 down)
    { axis: "B", src: "btnpair", pos: 15, neg: 14, sign: +1, max: 5 }, // hat right/left (15 right, 14 left)
];
// Twist (axes[2]) and throttle (axes[3]) are deliberately unmapped.
const JOY_DEADMAN_BTN = 0; // trigger, hold to move
const JOY_STOP_BTN = 1; // thumb button -> STOP
const JOY_DEADZONE = 0.25;
const JOY_TICK_MS = 50;
const JOY_SEG_S = 0.15; // each segment = ~0.15 s of motion at the chosen feed
const JOY_MIN_DELTA = 0.05; // deg; smallest segment sent
const JOY_PLANNER_SLOTS = 48; // g2core planner size; status qr = free slots
const JOY_MAX_QUEUED = 3; // send only if (48 - qr) <= this (~1-2 motion segments; jog = 2 lines)
const JOY_FEED_MIN = 100;
const JOY_FEED_MAX = 2000;
const JOY_FEED_DEFAULT = 900;
const JOY_FEED_KEY = "armweb.joyFeed";
const JOY_STATUS_MAX_AGE = 1000; // ms; ignore status older than this
const JOY_LINK_MAX_AGE = 4000; // ms; matches the link-health badge threshold

const joy = {
    enabled: false,
    armed: false, // false until stick has been centered once after enabling
    inflight: false,
    rr: -1,
    stopWas: false,
    padIndex: null,
    feed: joyLoadFeed(),
    map: null, // live key mapping (see joyDefaultMap)
    mapPadId: null, // gamepad id the live mapping belongs to
    dots: {}, // action -> activity dot element
    learn: null, // non-null while a learn-mode capture is active (driving suspended)
    note: "", // last mapping message (warnings etc.)
};
// --- key mapping (learn mode) ------------------------------------------------
// Live mapping, persisted per gamepad id. Joints: {kind:"axis", idx, dir} (dir =
// sign of the raw axis push that means "+") or {kind:"btnpair", pos, neg}, or null
// (unbound); plus invert. deadman/stop are button indices or null.
const JOY_MAP_KEY = "armweb.joyMap.";
const JOY_JOINT_ROWS = ["X", "Y", "Z", "A", "B"];
const JOY_LEARN_THRESH = 0.6; // |axis - baseline| needed to capture
const JOY_LEARN_REST = 0.3; // axis must come back within this of baseline
function joyMappable(ax) {
    return !DISABLED_AXES.includes(ax);
}
function joyDefaultMap() {
    const m = { deadman: JOY_DEADMAN_BTN, stop: JOY_STOP_BTN, joints: {} };
    for (const j of JOY_MAP) {
        if (!joyMappable(j.axis))
            continue;
        if (j.src === "axis")
            m.joints[j.axis] = { b: { kind: "axis", idx: j.idx, dir: j.sign < 0 ? -1 : 1 }, invert: false };
        else
            m.joints[j.axis] = { b: j.sign < 0 ? { kind: "btnpair", pos: j.neg, neg: j.pos } : { kind: "btnpair", pos: j.pos, neg: j.neg }, invert: false };
    }
    return m;
}
function joyIdx(v) {
    return Number.isInteger(v) && v >= 0 && v < 64 ? v : null;
}
// Validate untrusted/stored JSON into a safe mapping (never trust localStorage).
function joySanitizeMap(raw) {
    const m = { deadman: null, stop: null, joints: {} };
    if (!raw || typeof raw !== "object")
        return joyDefaultMap();
    m.deadman = joyIdx(raw.deadman);
    m.stop = joyIdx(raw.stop);
    for (const ax of JOY_JOINT_ROWS) {
        if (!joyMappable(ax))
            continue;
        const r = raw.joints && raw.joints[ax];
        let b = null;
        const rb = r && r.b;
        if (rb && rb.kind === "axis" && joyIdx(rb.idx) !== null && (rb.dir === 1 || rb.dir === -1))
            b = { kind: "axis", idx: rb.idx, dir: rb.dir };
        else if (rb && rb.kind === "btnpair" && joyIdx(rb.pos) !== null && joyIdx(rb.neg) !== null)
            b = { kind: "btnpair", pos: rb.pos, neg: rb.neg };
        m.joints[ax] = { b, invert: !!(r && r.invert) };
    }
    return m;
}
function joyLoadMap(padId) {
    try {
        const t = localStorage.getItem(JOY_MAP_KEY + padId);
        if (t !== null)
            return joySanitizeMap(JSON.parse(t));
    }
    catch (e) { /* storage unavailable or corrupt */ }
    return joyDefaultMap();
}
function joySaveMap() {
    if (joy.mapPadId === null)
        return;
    try {
        localStorage.setItem(JOY_MAP_KEY + joy.mapPadId, JSON.stringify(joy.map));
    }
    catch (e) { /* storage unavailable: mapping lasts until reload */ }
}
// After any mapping change: force a re-centre before driving, and don't let a
// button that is already held count as a fresh STOP press.
function joyMapChanged() {
    joy.armed = false;
    const pad = joyPad();
    joy.stopWas = !!(pad && joy.map.stop !== null && pad.buttons[joy.map.stop]?.pressed);
    joySaveMap();
    joyRenderMap();
}
function joyEnsureMap(pad) {
    const id = pad ? pad.id : null;
    if (id === joy.mapPadId && joy.map)
        return;
    if (id === null) {
        if (!joy.map)
            joy.map = joyDefaultMap();
        return;
    }
    joy.mapPadId = id;
    joy.map = joyLoadMap(id);
    joy.armed = false;
    joyLearnCancel("gamepad changed");
    joyRenderMap();
}
function joyControls(b) {
    if (b === null || b === undefined)
        return [];
    if (typeof b === "number")
        return ["b" + b];
    if (b.kind === "axis")
        return ["a" + b.idx];
    return ["b" + b.pos, "b" + b.neg];
}
function joyActionName(a) {
    return a === "deadman" ? "DEADMAN" : a === "stop" ? "STOP" : a;
}
function joyBindingOf(a) {
    return a === "deadman" ? joy.map.deadman : a === "stop" ? joy.map.stop : joy.map.joints[a]?.b ?? null;
}
function joyClearBinding(a) {
    if (a === "deadman")
        joy.map.deadman = null;
    else if (a === "stop")
        joy.map.stop = null;
    else if (joy.map.joints[a])
        joy.map.joints[a].b = null;
}
function joyDescribe(b) {
    if (b === null || b === undefined)
        return "unbound";
    if (typeof b === "number")
        return `button ${b}`;
    if (b.kind === "axis")
        return `axis ${b.idx}, pushed ${b.dir < 0 ? "negative" : "positive"} for +`;
    return `buttons ${b.pos} / ${b.neg} (+ / -)`;
}
// Commit a capture; warn about, and clear, any other action using the same control.
function joyCommit(action, binding) {
    const mine = joyControls(binding);
    const warns = [];
    for (const other of ["deadman", "stop", ...JOY_JOINT_ROWS.filter(joyMappable)]) {
        if (other === action)
            continue;
        const ob = joyBindingOf(other);
        const hit = joyControls(ob).filter((c) => mine.includes(c));
        if (hit.length) {
            joyClearBinding(other);
            warns.push(`${hit.map((c) => (c[0] === "a" ? "axis " : "button ") + c.slice(1)).join(", ")} was bound to ${joyActionName(other)} - now cleared`);
        }
    }
    if (action === "deadman")
        joy.map.deadman = binding;
    else if (action === "stop")
        joy.map.stop = binding;
    else
        joy.map.joints[action].b = binding;
    joy.note = `${joyActionName(action)} = ${joyDescribe(binding)}.` + (warns.length ? ` WARNING: ${warns.join("; ")}.` : "");
    joyMapChanged();
}
function joySetNote(msg) {
    joy.note = msg;
    const n = el("joy-note");
    if (n)
        n.textContent = msg;
}
function joyLearnStart(action) {
    const pad = joyPad();
    if (!pad) {
        joySetNote("no gamepad detected (press a button on the stick)");
        return;
    }
    joy.learn = {
        action,
        step: 1,
        phase: "capture",
        base: Array.from(pad.axes),
        held: new Set(pad.buttons.map((b, i) => (b.pressed ? i : -1)).filter((i) => i >= 0)),
        ignoreAx: new Set(),
        first: null,
        captured: null,
        msg: "",
    };
    joy.armed = false;
    joyRenderMap();
}
function joyLearnCancel(why) {
    if (!joy.learn)
        return;
    joy.learn = null;
    joy.armed = false;
    const pad = joyPad();
    joy.stopWas = !!(pad && joy.map.stop !== null && pad.buttons[joy.map.stop]?.pressed);
    if (why)
        joySetNote(`learn cancelled: ${why}`);
    joyRenderMap();
}
function joyLearnPrompt() {
    const L = joy.learn;
    if (!L)
        return "";
    const name = joyActionName(L.action);
    if (L.phase === "rest")
        return `Captured - release the control and let it return to rest...`;
    if (L.step === 2)
        return `Now the button for ${name} minus (Esc to cancel)` + (L.msg ? ` - ${L.msg}` : "");
    return `Move or press the control for ${name} now (Esc to cancel)` + (L.msg ? ` - ${L.msg}` : "");
}
// One learn-mode tick. Driving is suspended for as long as joy.learn is set.
function joyLearnTick(pad) {
    const L = joy.learn;
    const btnOnly = L.action === "deadman" || L.action === "stop" || L.step === 2;
    // buttons released since entry/last tick stop being "held" and may be captured later
    for (const i of Array.from(L.held))
        if (!pad.buttons[i]?.pressed)
            L.held.delete(i);
    for (const i of Array.from(L.ignoreAx))
        if (Math.abs((pad.axes[i] ?? 0) - (L.base[i] ?? 0)) < JOY_LEARN_REST)
            L.ignoreAx.delete(i);
    if (L.phase === "rest") {
        const c = L.captured;
        const atRest = c.kind === "axis"
            ? Math.abs((pad.axes[c.idx] ?? 0) - (L.base[c.idx] ?? 0)) < JOY_LEARN_REST
            : !pad.buttons[c.idx]?.pressed;
        if (!atRest)
            return;
        if (L.action !== "deadman" && L.action !== "stop" && c.kind === "button" && L.step === 1) {
            // button capture for a joint: ask for the minus button next
            L.first = c.idx;
            L.step = 2;
            L.phase = "capture";
            L.base = Array.from(pad.axes);
            L.held = new Set(pad.buttons.map((b, i) => (b.pressed ? i : -1)).filter((i) => i >= 0));
            L.captured = null;
            L.msg = "";
        }
        else {
            const a = L.action;
            let binding;
            if (a === "deadman" || a === "stop")
                binding = c.idx;
            else if (c.kind === "axis")
                binding = { kind: "axis", idx: c.idx, dir: c.dir };
            else
                binding = { kind: "btnpair", pos: L.first, neg: c.idx };
            joy.learn = null;
            joyCommit(a, binding);
            return;
        }
        joyRenderMap();
        return;
    }
    // capture phase: first axis past threshold, else first newly pressed button
    for (let i = 0; i < pad.axes.length; i++) {
        if (L.ignoreAx.has(i))
            continue;
        const d = (pad.axes[i] ?? 0) - (L.base[i] ?? 0);
        if (Math.abs(d) > JOY_LEARN_THRESH) {
            if (btnOnly) {
                L.ignoreAx.add(i);
                L.msg = `axis ${i} moved - ${joyActionName(L.action)} must be a button`;
                joyRenderMap();
                return;
            }
            L.captured = { kind: "axis", idx: i, dir: d > 0 ? 1 : -1 };
            L.phase = "rest";
            joyRenderMap();
            return;
        }
    }
    for (let i = 0; i < pad.buttons.length; i++) {
        if (pad.buttons[i]?.pressed && !L.held.has(i)) {
            if (L.step === 2 && i === L.first) {
                L.held.add(i);
                L.msg = `button ${i} is already the plus button, press a different one`;
                joyRenderMap();
                return;
            }
            L.captured = { kind: "button", idx: i };
            L.phase = "rest";
            joyRenderMap();
            return;
        }
    }
}

function joyClampFeed(v) {
    v = Math.round(Number(v));
    if (!isFinite(v))
        return JOY_FEED_DEFAULT;
    return Math.min(JOY_FEED_MAX, Math.max(JOY_FEED_MIN, v));
}
function joyLoadFeed() {
    try {
        const v = localStorage.getItem(JOY_FEED_KEY);
        if (v !== null)
            return joyClampFeed(v);
    }
    catch (e) { /* storage unavailable */ }
    return JOY_FEED_DEFAULT;
}
function joySaveFeed(v) {
    try {
        localStorage.setItem(JOY_FEED_KEY, String(v));
    }
    catch (e) { /* storage unavailable */ }
}

function buildJoystickPanel() {
    const sec = el("joystick");
    sec.innerHTML =
        `<h2>joystick</h2>` +
            `<label><input id="joy-en" type="checkbox" /> Enable joystick</label>` +
            `<div class="joyspeed"><label for="joy-feed">speed (deg/min)</label> ` +
            `<input id="joy-feed" type="range" min="${JOY_FEED_MIN}" max="${JOY_FEED_MAX}" step="50" value="${joy.feed}" /> ` +
            `<span id="joy-feed-val">${joy.feed}</span></div>` +
            `<div id="joy-status" class="summary"></div>` +
            `<div id="joy-pad" class="hint"></div>` +
            `<div id="joy-learn" class="joylearn" hidden></div>` +
            `<table id="joy-map" class="joymap"></table>` +
            `<div id="joy-note" class="hint"></div>` +
            `<div class="joymapbtns"><button id="joy-reset">Reset to defaults</button> <button id="joy-copy">Copy mapping</button></div>` +
            `<textarea id="joy-json" class="joyjson" rows="8" readonly hidden></textarea>` +
            `<pre id="joy-raw" class="joyraw"></pre>` +
            `<p class="hint">Hand on arm power. Hold the DEADMAN button to move; STOP button stops. Base disabled. Use Map to rebind controls.</p>`;
    joyEnsureMap(null);
    el("joy-reset").addEventListener("click", () => {
        joyLearnCancel();
        joy.map = joyDefaultMap();
        joySetNote("mapping reset to defaults");
        joyMapChanged();
    });
    el("joy-copy").addEventListener("click", () => {
        const ta = el("joy-json");
        ta.value = JSON.stringify({ gamepad: joy.mapPadId, mapping: joy.map }, null, 2);
        ta.hidden = false;
        ta.focus();
        ta.select();
        try {
            navigator.clipboard?.writeText(ta.value).catch(() => { });
        }
        catch (e) { /* clipboard unavailable; textarea is selected for manual copy */ }
    });
    joyRenderMap();
    el("joy-feed").addEventListener("input", () => {
        joy.feed = joyClampFeed(el("joy-feed").value);
        el("joy-feed-val").textContent = String(joy.feed);
        joySaveFeed(joy.feed);
    });
    el("joy-en").addEventListener("change", () => {
        if (el("joy-en").checked)
            joyEnable();
        else
            joyDisable("disabled by user");
    });
}

function joyRenderMap() {
    const tbl = el("joy-map");
    if (!tbl)
        return;
    tbl.innerHTML = "";
    joy.dots = {};
    const mk = (tag, cls, txt) => {
        const e = document.createElement(tag);
        if (cls)
            e.className = cls;
        if (txt !== undefined)
            e.textContent = txt;
        return e;
    };
    const hdr = document.createElement("tr");
    for (const h of ["", "action", "binding", "invert", ""])
        hdr.append(mk("th", "", h));
    tbl.append(hdr);
    const learning = !!joy.learn;
    for (const a of ["deadman", "stop", ...JOY_JOINT_ROWS]) {
        const tr = document.createElement("tr");
        const dotc = mk("td");
        const dotEl = mk("span", "dot");
        dotc.append(dotEl);
        const name = mk("td", "", joyActionName(a));
        const bind = mk("td");
        const inv = mk("td");
        const acts = mk("td");
        if (!joyMappable(a)) {
            tr.className = "joydis";
            bind.textContent = "disabled (driver #1 wiring)";
        }
        else {
            joy.dots[a] = dotEl;
            bind.textContent = joyDescribe(joyBindingOf(a));
            if (joy.learn && joy.learn.action === a) {
                tr.className = "joylearning";
                bind.textContent = "learning...";
            }
            const map = mk("button", "", "Map");
            map.disabled = learning;
            map.addEventListener("click", () => joyLearnStart(a));
            acts.append(map);
            if (a !== "deadman" && a !== "stop") {
                const cb = document.createElement("input");
                cb.type = "checkbox";
                cb.checked = !!joy.map.joints[a].invert;
                cb.disabled = learning;
                cb.addEventListener("change", () => {
                    joy.map.joints[a].invert = cb.checked;
                    joySetNote(`${a} invert ${cb.checked ? "on" : "off"}`);
                    joyMapChanged();
                });
                inv.append(cb);
            }
            const clr = mk("button", "", "clear");
            clr.disabled = learning;
            clr.addEventListener("click", () => {
                joyClearBinding(a);
                if (a !== "deadman" && a !== "stop")
                    joy.map.joints[a].invert = false;
                joySetNote(`${joyActionName(a)} cleared`);
                joyMapChanged();
            });
            acts.append(" ", clr);
        }
        tr.append(dotc, name, bind, inv, acts);
        tbl.append(tr);
    }
    const lp = el("joy-learn");
    lp.hidden = !learning;
    lp.textContent = joyLearnPrompt();
    const n = el("joy-note");
    if (n)
        n.textContent = joy.note;
}

function joyPad() {
    const pads = (navigator.getGamepads ? navigator.getGamepads() : []);
    const list = Array.from(pads).filter((p) => p && p.connected);
    return list.find((p) => /thrustmaster|flight|044f/i.test(p.id)) ?? list[0] ?? null;
}

function joySetStatus(msg, bad) {
    const s = el("joy-status");
    s.textContent = msg;
    s.className = "summary" + (bad ? " lh-bad" : "");
}

function joyEnable() {
    if (!joyPad()) {
        el("joy-en").checked = false;
        joySetStatus("no gamepad detected (press a button on the stick)", true);
        return;
    }
    joy.enabled = true;
    joy.armed = false;
    joy.inflight = false;
    joy.rr = -1;
    joySetStatus("enabled - centre the stick and release hat/deadman to arm");
}

function joyDisable(reason) {
    const was = joy.enabled;
    joy.enabled = false;
    joy.armed = false;
    el("joy-en").checked = false;
    if (was || reason)
        joySetStatus(`OFF: ${reason}`, was && reason !== "disabled by user");
}

// Joints driven by the live mapping, in round-robin order.
const JOY_DRIVE = JOY_JOINT_ROWS.filter(joyMappable);

// Current command per joint (JOY_DRIVE order): signed deflection -1..1 after
// deadzone (0 = none). Axis bindings are proportional; button bindings are +/-1.
function joyDeltas(pad) {
    return JOY_DRIVE.map((ax) => {
        const j = joy.map.joints[ax];
        const b = j && j.b;
        let v = 0;
        if (b && b.kind === "axis") {
            const a = pad.axes[b.idx] ?? 0;
            if (Math.abs(a) > JOY_DEADZONE) {
                const d = Math.min(1, (Math.abs(a) - JOY_DEADZONE) / (1 - JOY_DEADZONE));
                v = Math.sign(a) * b.dir * d;
            }
        }
        else if (b) {
            const p = pad.buttons[b.pos]?.pressed ? 1 : 0;
            const n = pad.buttons[b.neg]?.pressed ? 1 : 0;
            v = p - n;
        }
        if (j && j.invert)
            v = -v;
        // Never send a disabled axis, even if someone edits the mapping.
        if (DISABLED_AXES.includes(ax))
            v = 0;
        return v;
    });
}

function joyRender(pad) {
    if (!pad) {
        el("joy-pad").textContent = "no gamepad detected (press a button on the stick)";
        el("joy-raw").textContent = "";
        return;
    }
    el("joy-pad").textContent = `gamepad ${pad.index}: ${pad.id} (mapping: ${pad.mapping || "none"})`;
    const ax = pad.axes.map((a, i) => `a${i}:${a.toFixed(2)}`).join("  ");
    const bt = pad.buttons.map((b, i) => `b${i}:${b.pressed ? "1" : "0"}${b.value > 0 && b.value < 1 ? "(" + b.value.toFixed(2) + ")" : ""}`).join(" ");
    el("joy-raw").textContent = `axes   ${ax}\nbuttons ${bt}`;
    joyUpdateDots(pad);
}

// Live activity dots: lit when the control is currently active (pressed/pushed).
function joyUpdateDots(pad) {
    const on = {};
    on.deadman = joy.map.deadman !== null && !!pad.buttons[joy.map.deadman]?.pressed;
    on.stop = joy.map.stop !== null && !!pad.buttons[joy.map.stop]?.pressed;
    for (const ax of JOY_JOINT_ROWS) {
        const b = joy.map.joints[ax]?.b;
        if (!b)
            on[ax] = false;
        else if (b.kind === "axis")
            on[ax] = Math.abs(pad.axes[b.idx] ?? 0) > JOY_DEADZONE;
        else
            on[ax] = !!pad.buttons[b.pos]?.pressed || !!pad.buttons[b.neg]?.pressed;
    }
    for (const k in joy.dots)
        joy.dots[k].className = "dot" + (on[k] ? " on" : "");
}

async function joyTick() {
    const pad = joyPad();
    joyEnsureMap(pad);
    joyRender(pad);
    // --- learn mode: driving is suspended (no jogs, no stick STOP) until it ends ---
    if (joy.learn) {
        if (!pad || document.hidden)
            joyLearnCancel(!pad ? "gamepad disconnected" : "page hidden");
        else {
            joyLearnTick(pad);
            if (joy.learn)
                return;
        }
    }
    if (!joy.enabled)
        return;
    // --- safety auto-disable ---
    if (!pad)
        return joyDisable("gamepad disconnected");
    if (document.hidden)
        return joyDisable("page hidden");
    if (!lastNetOk || Date.now() - lastNetOk > JOY_LINK_MAX_AGE)
        return joyDisable("Pi unreachable (link health)");
    // --- thumb = STOP (edge-triggered; works even before armed) ---
    const stopNow = joy.map.stop !== null && !!pad.buttons[joy.map.stop]?.pressed;
    if (stopNow && !joy.stopWas) {
        doStop();
        joySetStatus("STOP sent");
    }
    joy.stopWas = stopNow;
    const deltas = joyDeltas(pad);
    // --- must see centre once after enabling ---
    if (!joy.armed) {
        if (deltas.every((d) => d === 0))
            joy.armed = true;
        else
            return;
    }
    // --- deadman ---
    if (joy.map.deadman === null) {
        joySetStatus("DEADMAN unbound - map it to move", true);
        return;
    }
    if (!pad.buttons[joy.map.deadman]?.pressed) {
        joySetStatus("armed - hold deadman to move");
        return;
    }
    // --- round-robin among deflected axes (segments still blend in the planner) ---
    let pick = -1;
    for (let k = 1; k <= JOY_DRIVE.length; k++) {
        const i = (joy.rr + k) % JOY_DRIVE.length;
        if (deltas[i] !== 0) {
            pick = i;
            break;
        }
    }
    if (pick < 0) {
        joySetStatus("armed - deadman held, controls centred");
        return;
    }
    // --- pacing: never more than one request in flight ---
    if (joy.inflight)
        return;
    const fresh = lastSr && Date.now() - lastSrAt <= JOY_STATUS_MAX_AGE;
    const queued = fresh && typeof lastSr.qr === "number" ? JOY_PLANNER_SLOTS - lastSr.qr : null;
    if (queued === null || queued > JOY_MAX_QUEUED) {
        // Stale status or planner has enough queued: re-poll fresh state (next tick re-checks).
        joy.inflight = true;
        try {
            noteStatus(await api.getState());
        }
        catch (e) {
            joyDisable(`request failed: ${e}`);
        }
        finally {
            joy.inflight = false;
        }
        return;
    }
    joy.rr = pick;
    const axis = JOY_DRIVE[pick];
    const mag = Math.max(JOY_MIN_DELTA, (joy.feed / 60) * JOY_SEG_S * Math.abs(deltas[pick]));
    const delta = Math.round(Math.sign(deltas[pick]) * mag * 100) / 100;
    joy.inflight = true;
    try {
        await api.jog(axis, delta, joy.feed);
        joySetStatus(`jog ${axis} ${delta > 0 ? "+" : ""}${delta} @ ${joy.feed}`);
        // Fresh status right after each jog so the next decision sees the new queue depth.
        noteStatus(await api.getState());
    }
    catch (e) {
        joyDisable(`request failed: ${e}`);
    }
    finally {
        joy.inflight = false;
    }
}

buildJoystickPanel();
document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && joy.learn) {
        e.preventDefault();
        joyLearnCancel("cancelled");
    }
});
window.addEventListener("gamepaddisconnected", () => {
    joyLearnCancel("gamepad disconnected");
    if (joy.enabled)
        joyDisable("gamepad disconnected");
});
document.addEventListener("visibilitychange", () => {
    if (document.hidden)
        joyLearnCancel("page hidden");
    if (document.hidden && joy.enabled)
        joyDisable("page hidden");
});
setInterval(joyTick, JOY_TICK_MS);
//# sourceMappingURL=main.js.map