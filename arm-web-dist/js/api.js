// Typed client for the arm-core HTTP+JSON API.
//
// These interfaces mirror the Rust `arm-core-types` crate (the wire contract).
// They're kept in sync by hand: this is the TS client, so it can't import the
// Rust types. If arm-core were ever consumed by a Rust/WASM client instead, that
// client would depend on `arm-core-types` directly and skip this duplication.
// arm-core base URL. Defaults to the Pi over mDNS; overridable in the UI and
// remembered in localStorage so a debug session sticks.
const DEFAULT_BASE = ""; // same origin: the gateway serves this page
let base = localStorage.getItem("armcore.base") ?? DEFAULT_BASE;
export function getBase() {
    return base;
}
export function setBase(url) {
    base = url.replace(/\/+$/, "");
    localStorage.setItem("armcore.base", base);
}
async function req(method, path, body) {
    const res = await fetch(base + path, {
        method,
        headers: body !== undefined ? { "Content-Type": "application/json" } : undefined,
        body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    const data = text ? JSON.parse(text) : {};
    if (!res.ok) {
        const msg = typeof data === "object" && data !== null && "error" in data
            ? String(data.error)
            : `HTTP ${res.status}`;
        throw new Error(msg);
    }
    return data;
}
export const getState = () => req("GET", "/state");
export const getInputs = () => req("GET", "/inputs");
export const jog = (axis, delta, feed) => req("POST", "/jog", feed !== undefined ? { axis, delta, feed } : { axis, delta });
export const home = (axis) => req("POST", "/home", { axis });
export const stop = () => req("POST", "/stop");
export const raw = (line) => req("POST", "/raw", { line });
//# sourceMappingURL=api.js.map