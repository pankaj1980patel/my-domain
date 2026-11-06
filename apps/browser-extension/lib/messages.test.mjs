// node apps/browser-extension/lib/messages.test.mjs  (no dependencies)

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as M from "./messages.js";

const tests = [];
const test = (name, fn) => tests.push([name, fn]);

test("links are http(s) only and trimmed", () => {
  assert.equal(M.webUrl("  https://example.com/a?b#c "), "https://example.com/a?b#c");
  assert.equal(M.webUrl("HTTP://Example.com"), "HTTP://Example.com");
  for (const bad of ["javascript:alert(1)", "file:///etc/passwd", "chrome://settings", "about:blank", "", "http://a b", "x".repeat(5000), null]) {
    assert.equal(M.webUrl(bad), null, String(bad).slice(0, 30));
  }
});

test("page and link messages", () => {
  assert.deepEqual(M.sendUrl("node-1", "https://a.io/x", "  A \n title "), {
    type: "send_url",
    device: "node-1",
    url: "https://a.io/x",
    title: "A title",
  });
  assert.deepEqual(M.sendUrl("node-1", "https://a.io", "   "), { type: "send_url", device: "node-1", url: "https://a.io" });
  assert.throws(() => M.sendUrl("../x", "https://a.io"), /bad_device/);
  assert.throws(() => M.openUrl("n", "javascript:1"), /bad_url/);
  assert.deepEqual(M.readLater("https://a.io", "T"), { type: "read_later_add", url: "https://a.io", title: "T" });
  assert.deepEqual(M.downloadOn("n", "https://a.io/f.zip"), { type: "download_on", device: "n", url: "https://a.io/f.zip" });
});

test("text messages", () => {
  assert.deepEqual(M.sendText("n", "hello"), { type: "send_text", device: "n", text: "hello" });
  assert.throws(() => M.sendText("n", "  "), /no_text/);
  assert.throws(() => M.sendText("n", "x".repeat(M.MAX_TEXT + 1)), /too_long/);
});

test("files go in pieces under the 1 MB message cap", () => {
  const bytes = new Uint8Array(M.MAX_CHUNK * 2 + 10).map((_, i) => i % 251);
  const msgs = M.fileMessages("n", "cat.png", bytes, "upload-1234");
  assert.equal(msgs.length, 3);
  assert.deepEqual(
    msgs.map((m) => [m.offset, m.done]),
    [
      [0, false],
      [M.MAX_CHUNK, false],
      [M.MAX_CHUNK * 2, true],
    ],
  );
  for (const m of msgs) {
    assert.ok(JSON.stringify(m).length < 1024 * 1024, "fits a message");
    assert.equal(m.upload, "upload-1234");
  }
  const back = Buffer.concat(msgs.map((m) => Buffer.from(m.data, "base64")));
  assert.deepEqual(new Uint8Array(back), bytes);
  assert.equal(M.fileMessages("n", "a", new Uint8Array([1])).length, 1);
  assert.throws(() => M.fileMessages("n", "a", new Uint8Array(0)), /empty/);
  assert.match(M.uploadId(), /^[A-Za-z0-9_-]{16}$/);
});

test("picture names", () => {
  assert.equal(M.imageName("https://a.io/img/cat.jpg?x=1", "image/jpeg"), "cat.jpg");
  assert.equal(M.imageName("https://a.io/img/photo", "image/webp"), "photo.webp");
  assert.equal(M.imageName("data:image/png;base64,AAAA", "image/png"), "image.png");
  assert.equal(M.imageName("https://a.io/..%2F..%2Fevil", "image/png"), "_.._evil.png");
});

test("menu ids", () => {
  assert.equal(M.menuId("open_on", "node-1"), "open_on|node-1");
  assert.deepEqual(M.parseMenuId("open_on|node-1"), { action: "open_on", node: "node-1" });
  assert.equal(M.parseMenuId("rm|node-1"), null);
  assert.equal(M.parseMenuId("open_on|none x"), null);
  assert.equal(M.parseMenuId("read_later"), null);
});

test("menu clicks build the right message", () => {
  const tab = { url: "https://site.io/page", title: "Page" };
  assert.deepEqual(M.forMenu("send_page", "n", { pageUrl: tab.url }, tab), {
    type: "send_url",
    device: "n",
    url: "https://site.io/page",
    title: "Page",
  });
  assert.equal(M.forMenu("send_link", "n", { linkUrl: "https://l.io", pageUrl: tab.url }, tab).url, "https://l.io");
  assert.equal(M.forMenu("send_text", "n", { selectionText: "hi" }, tab).text, "hi");
  assert.equal(M.forMenu("download_on", "n", { srcUrl: "https://i.io/a.png", pageUrl: tab.url }, tab).url, "https://i.io/a.png");
  assert.equal(M.forMenu("open_on", "n", { pageUrl: tab.url }, tab).url, tab.url);
  assert.equal(M.forMenu("send_image", "n", {}, tab), null);
});

test("error codes and icons", () => {
  assert.equal(M.errorKey("app_not_running"), "errAppNotRunning");
  assert.equal(M.errorKey("whatever"), "errFailed");
  assert.equal(M.iconGroup("mac_laptop"), "laptop");
  assert.equal(M.iconGroup("android_foldable"), "phone");
  assert.equal(M.iconGroup("imac"), "desktop");
  assert.equal(M.iconGroup("nonsense"), "unknown");
});

test("every locale has every message the code uses", () => {
  const dir = new URL("../", import.meta.url);
  const en = JSON.parse(readFileSync(new URL("_locales/en/messages.json", dir), "utf8"));
  const hi = JSON.parse(readFileSync(new URL("_locales/hi/messages.json", dir), "utf8"));
  assert.deepEqual(Object.keys(hi).sort(), Object.keys(en).sort());
  const code = ["background.js", "popup.js", "popup.html", "lib/messages.js"].map((f) => readFileSync(new URL(f, dir), "utf8")).join("\n");
  for (const m of code.matchAll(/(?:(?<![\w.])t\(|data-i18n(?:-title)?=)"([A-Za-z]+)"/g)) {
    assert.ok(en[m[1]], `missing ${m[1]}`);
  }
  for (const m of code.matchAll(/return "(err[A-Za-z]+)"/g)) assert.ok(en[m[1]], `missing ${m[1]}`);
  for (const m of readFileSync(new URL("manifest.json", dir), "utf8").matchAll(/__MSG_([A-Za-z]+)__/g)) {
    assert.ok(en[m[1]], `missing ${m[1]}`);
  }
});

let failed = 0;
for (const [name, fn] of tests) {
  try {
    fn();
    console.log(`ok   ${name}`);
  } catch (e) {
    failed++;
    console.log(`FAIL ${name}\n     ${e.message}`);
  }
}
console.log(`${tests.length - failed}/${tests.length} passed`);
process.exit(failed ? 1 : 0);
