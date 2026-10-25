// The browser and Node checks use the same small host bridge. It works with
// wasm32 pointers (Numbers) and wasm64 pointers (BigInts).
export async function loadTerminal(bytes) {
    let result;
    const { instance } = await WebAssembly.instantiate(bytes, {
        env: {
            display(ptr, len, error) {
                // Always acquire a fresh view: any Wasm call can grow memory.
                const bytes = new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len));
                result = { text: new TextDecoder().decode(bytes), length: Number(len), error };
            },
            log(ptr, len) {
                const bytes = new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len));
                console.log('[wasm]', new TextDecoder().decode(bytes));
            },
        },
    });

    return (cols, rows, text) => {
        const bytes = new TextEncoder().encode(text);
        const ptr = instance.exports.input_alloc(bytes.length);
        try {
            new Uint8Array(instance.exports.memory.buffer, Number(ptr), bytes.length).set(bytes);
            instance.exports.run(cols, rows, ptr, bytes.length);
            if (result.error) throw new Error(result.text);
            return { text: result.text, inputLength: bytes.length, outputLength: result.length };
        } finally {
            instance.exports.input_free(ptr, bytes.length);
        }
    };
}

// Parse escape sequences in the input string (e.g. \x1b, \r, \n).
// Keep upstream's replacement order, including its treatment of backslashes.
export function parseEscapes(str) {
    return str
        .replace(/\\x([0-9a-fA-F]{2})/g, (_, hex) => String.fromCharCode(parseInt(hex, 16)))
        .replace(/\\r/g, '\r')
        .replace(/\\n/g, '\n')
        .replace(/\\t/g, '\t')
        .replace(/\\\\/g, '\\');
}
