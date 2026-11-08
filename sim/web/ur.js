// The simulator's WebAssembly module (built from sim/src/, see
// sim/src/web.rs), wrapped for JavaScript: the page (index.html) and the
// smoke test (smoke.mjs) both load it through this.
//
// The module's interface is plain numbers and pointers into its memory,
// so this is all the glue there is: text goes in by writing UTF-8 into a
// buffer the module hands out, and comes out through `print`.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// What `print`'s `kind` is (`web::Text`).
export const LOG = 0;
export const ERROR = 1;
export const STATUS = 2;
export const THEMES = 3;

// Instantiates the module from its bytes, and boots the watch, which
// runs sim/web/start.txt unless `start` is false. `print(kind, text)` gets
// every line the simulator prints; `download(name, bytes)` every file it
// writes (screenshots).
export async function load(bytes, { print, download, start = true }) {
  let memory;
  const text = (pointer, length) =>
    decoder.decode(new Uint8Array(memory.buffer, pointer, length));
  const { instance } = await WebAssembly.instantiate(bytes, {
    env: {
      ur_print: (kind, pointer, length) => print(kind, text(pointer, length)),
      ur_download: (name, nameLength, data, length) =>
        // A copy: the module's memory may move once it returns.
        download(text(name, nameLength), new Uint8Array(memory.buffer, data, length).slice()),
      ur_now_ms: () => Date.now(),
      // `getTimezoneOffset` is west-positive; the watch's is east-positive.
      ur_utc_offset_minutes: (ms) => -new Date(ms).getTimezoneOffset(),
    },
  });
  const ur = instance.exports;
  memory = ur.memory;

  // Puts `string` where the next call reads its text.
  const write = (string) => {
    const bytes = encoder.encode(string);
    new Uint8Array(memory.buffer, ur.ur_buffer(bytes.length), bytes.length).set(bytes);
  };

  ur.ur_boot(start);
  const width = ur.ur_width();
  const height = ur.ur_height();
  return {
    width,
    height,
    // One command, as typed into the terminal (`help` lists them).
    command(line) {
      write(line);
      ur.ur_command();
    },
    // A whole script, stopping at its first error.
    script(text) {
      write(text);
      ur.ur_script();
    },
    // A `KeyboardEvent.key`; whether the watch has a use for it.
    key(name) {
      write(name);
      return ur.ur_key() !== 0;
    },
    // The pointer, in the drawn watch's pixels: "down", "move", or "up".
    pointer(kind, x, y) {
      ur.ur_pointer({ down: 0, move: 1, up: 2 }[kind], x, y);
    },
    advance(ms) {
      ur.ur_advance(ms);
    },
    // The watch, drawn: `width` × `height` RGBA pixels. A view into the
    // module's memory, valid until the next call.
    frame() {
      return new Uint8ClampedArray(memory.buffer, ur.ur_frame(), width * height * 4);
    },
    status() {
      ur.ur_status();
    },
    themes() {
      ur.ur_themes();
    },
  };
}
