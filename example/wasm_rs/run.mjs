import { readFile } from "node:fs/promises";
import assert from "node:assert/strict";
import { loadTerminal, parseEscapes } from "./terminal.mjs";

const module = await WebAssembly.compile(await readFile(process.argv[2]));
// Debug Ghostty builds use env.log; release builds normally have no imports.
for (const entry of WebAssembly.Module.imports(module)) {
    assert.equal(`${entry.module}.${entry.name}:${entry.kind}`, "env.log:function");
}
const instance = await WebAssembly.instantiate(module, {
    env: {
        log(ptr, len) {
            // Memory64 passes pointers and lengths as BigInts. Reacquire
            // the buffer each time because library calls can grow memory.
            const bytes = new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len));
            console.error(new TextDecoder().decode(bytes));
        },
    },
});
instance.exports.run();
console.log("Wasm ABI, terminal, callbacks, and allocator checks passed");

// Exercise the exact bridge used by the browser, including the default input
// in the page. Keep the expected text independent of the implementation.
const format = await loadTerminal(await readFile(process.argv[3]));
const html = await readFile(new URL("./index.html", import.meta.url), "utf8");
const input = parseEscapes(html.match(/<textarea[^>]*>(.*?)<\/textarea>/s)[1]);
assert.equal(format(80, 24, input).text,
    "Hello, World!\nGreen Bold and Underline\nLine 3: Overwritten!");
assert.deepEqual(format(13, 7, "hé\r\n世"), { text: "hé\n世", inputLength: 8, outputLength: 7 });
assert.equal(format(3, 2, "abcdef").text, "abc\ndef");
assert.equal(format(80, 24, "").text, "");
assert.equal(format(80, 24, "\rabc".repeat(70000)).text, "abc");
assert.equal(format(80, 24, "again").text, "again");
// Upstream's HTML has min/max hints, but Run does not enforce them.
assert.equal(format(501, 24, "x").text, "x");
assert.equal(parseEscapes(String.raw`\\n\t\x1b`), "\\\n\t\x1b");
console.log("Upstream terminal demo checks passed");
