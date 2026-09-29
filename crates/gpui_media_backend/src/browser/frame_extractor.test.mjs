import test from "node:test";
import assert from "node:assert/strict";
import { extractFrame } from "./frame_extractor.js";

class Video extends EventTarget {
    readyState = 0;
    duration = 10;
    seeking = false;
    error = null;
    time = 0;
    seeks = 0;
    listeners = new Set();
    get currentTime() { return this.time; }
    set currentTime(value) { this.seeks++; this.time = value; this.seeking = true; }
    addEventListener(name, listener) { super.addEventListener(name, listener); this.listeners.add(name); }
    removeEventListener(name, listener) { super.removeEventListener(name, listener); this.listeners.delete(name); }
    emit(name) { this.dispatchEvent(new Event(name)); }
}
globalThis.VideoFrame = class {
    constructor(video) { this.timestamp = video.currentTime * 1_000_000; }
};

test("initial frame waits for decoded data without seeking or playing", async () => {
    const video = new Video();
    let resolved = false;
    const result = extractFrame(video, 0, 1000).then(frame => { resolved = true; return frame; });
    video.readyState = 1;
    video.emit("loadedmetadata");
    await Promise.resolve();
    assert.equal(resolved, false);
    video.readyState = 2;
    video.emit("loadeddata");
    assert.equal((await result).timestamp, 0);
    assert.equal(video.seeks, 0);
    assert.equal(video.listeners.size, 0);
});

test("seek waits for completion and repeated timestamp completes immediately", async () => {
    const video = new Video();
    video.readyState = 2;
    let resolved = false;
    const result = extractFrame(video, 3, 1000).then(frame => { resolved = true; return frame; });
    video.emit("loadeddata");
    await Promise.resolve();
    assert.equal(resolved, false);
    video.seeking = false;
    video.emit("seeked");
    assert.equal((await result).timestamp, 3_000_000);
    assert.equal((await extractFrame(video, 3, 1000)).timestamp, 3_000_000);
    assert.equal(video.seeks, 1);
    assert.equal(video.listeners.size, 0);
});

test("timeout and decode errors remove every listener", async () => {
    const video = new Video();
    await assert.rejects(extractFrame(video, 0, 1), { name: "TimeoutError" });
    assert.equal(video.listeners.size, 0);
    const result = extractFrame(video, 0, 1000);
    video.error = { message: "unsupported codec" };
    video.emit("error");
    await assert.rejects(result, /unsupported codec/);
    assert.equal(video.listeners.size, 0);
});

test("positions beyond duration fail without changing the video time", async () => {
    const video = new Video();
    video.readyState = 2;
    await assert.rejects(extractFrame(video, 12, 1000), RangeError);
    assert.equal(video.seeks, 0);
    assert.equal(video.listeners.size, 0);
});

test("a source that clamps a seek must not return the wrong frame", async () => {
    const video = new Video();
    video.readyState = 2;
    const result = extractFrame(video, 1, 1000);
    video.time = 0;
    video.seeking = false;
    video.emit("seeked");
    await assert.rejects(result, /cannot seek/);
    assert.equal(video.listeners.size, 0);
});
