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
    // The real panel's colours (true, the default), or those sent to it.
    mimic(on) {
      ur.ur_mimic(on);
    },
    // The watch, drawn: `width` × `height` RGBA pixels. A view into the
    // module's memory, valid until the next call.
    frame() {
      return new Uint8ClampedArray(memory.buffer, ur.ur_frame(), width * height * 4);
    },
    status() {
      ur.ur_status();
    },
    // Prints the colour schemes, as a THEMES text (see `schemes`).
    themes() {
      ur.ur_themes();
    },
    // Whether the watch shows the day's side now (by the sun, or forced
    // by a scheme picked).
    daytime() {
      return ur.ur_daytime() !== 0;
    },
  };
}

// ==============================================================================
// Links to a theme
// ==============================================================================
//
// The page takes its theme from its URL's query, so a theme's README
// can link to the page showing it (https://jonhoo.github.io/ur/sim/):
//
// - `?theme=IDENT`: a theme, by its identifier (`src/themes/IDENT/`,
//   `theme-IDENT`), in its first scheme for the side the watch shows
//   now, or for its other side if it has none for this one;
// - `?scheme=NAME`: a scheme, by its name in any case, as the `theme`
//   command takes it (`?scheme=tokyo+night`); with `theme` too, only if
//   it's one of that theme's;
// - `?side=night|day`: with `theme`, that side's first scheme rather
//   than the one the sun says; alone, the build's default for that side.
//
// The query rather than the hash: it's what a link's settings usually
// are, editing it in the address bar reloads the page (a hash change
// wouldn't, without more code to follow it), and Pages serves the same
// file whatever the query, as its URLs are all relative. Anything else
// in the query is left alone.
//
// It's all pure, apart from the watch's side, so smoke.mjs checks it
// as the page uses it, against the module's own list of schemes.

// A THEMES text, a scheme a line: `{ ident, theme, side, scheme }`.
export function schemes(text) {
  return text.split("\n").map((line) => {
    const [ident, theme, side, scheme] = line.split("\t");
    return { ident, theme, side, scheme };
  });
}

// ASCII letters only, as `theme NAME` matches (Rust's
// `eq_ignore_ascii_case`): `toLowerCase` would also fold, say, the
// Kelvin sign into a K.
const fold = (text) => text.replace(/[A-Z]/g, (c) => c.toLowerCase());

// The scheme (one of `list`, from `schemes`) a URL's query asks for, or
// undefined for the default (the sun's). Whatever the query says, the
// answer is one of the list's or nothing: a value that isn't built in is
// passed to `warn` and left out, never an error. `daytime`: the side
// the watch shows now.
export function chosen(query, list, daytime, warn) {
  const params = new URLSearchParams(query);
  const get = (key) => (params.has(key) ? fold(params.get(key)) : null);

  let side = get("side");
  if (side !== null && side !== "day" && side !== "night") {
    warn(`side=${params.get("side")}: neither day nor night, so the sun's`);
    side = null;
  }
  const day = side === null ? daytime : side === "day";

  let theme = get("theme");
  if (theme !== null && !list.some((s) => s.ident === theme)) {
    const idents = [...new Set(list.map((s) => s.ident))].join(", ");
    warn(`theme=${params.get("theme")}: no such theme built in (${idents})`);
    theme = null;
  }

  const scheme = get("scheme");
  if (scheme !== null) {
    const found = list.find((s) => fold(s.scheme) === scheme);
    if (!found) {
      warn(`scheme=${params.get("scheme")}: no such scheme built in`);
    } else if (theme !== null && found.ident !== theme) {
      warn(`scheme=${params.get("scheme")}: not one of ${theme}'s schemes, so its first`);
    } else {
      return found;
    }
  }

  // A theme's, or with only a side, the build's: that side's first is
  // the firmware's default for it.
  if (theme === null && side === null) return undefined;
  const its = theme === null ? list : list.filter((s) => s.ident === theme);
  return its.find((s) => (s.side === "day") === day) ?? its[0];
}

// `url` (a URL), with its query naming `scheme` (one of `schemes`'), or
// none for the default: what the page's address becomes when the wearer
// picks one, so it's always a link to what it shows. Both the theme and
// the scheme, as the theme alone would depend on the time of day.
export function linking(url, scheme) {
  const link = new URL(url);
  for (const key of ["theme", "scheme", "side"]) link.searchParams.delete(key);
  if (scheme) {
    link.searchParams.set("theme", scheme.ident);
    link.searchParams.set("scheme", scheme.scheme);
  }
  return link;
}
