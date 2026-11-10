// A headless check of the browser's simulator, in node (see check.sh):
//
//     node web/smoke.mjs ur_sim.wasm OUT [SCRIPT ...]
//
// First, it boots the watch as the page does and drives it as a wearer
// would, through the same functions the page calls (ur.js), checking
// that each input reaches the watch. Then it runs each SCRIPT on a watch
// of its own, booted bare (as the command line's), and writes its
// screenshots to OUT/SCRIPT-NAME/, for check.sh to compare with the
// command line's. Exits non-zero at the first thing that's wrong.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { ERROR, LOG, STATUS, THEMES, load } from "./ur.js";

const [wasmPath, out, ...scripts] = process.argv.slice(2);
if (!out) {
  console.error("usage: node smoke.mjs ur_sim.wasm OUT [SCRIPT ...]");
  process.exit(2);
}
const bytes = readFileSync(wasmPath);

function fail(message, lines) {
  console.error(`smoke: ${message}`);
  if (lines) console.error(lines.join("\n"));
  process.exit(1);
}

// A watch, with what it prints collected by kind.
async function watch({ start, files }) {
  const printed = { [LOG]: [], [ERROR]: [], [STATUS]: [], [THEMES]: [] };
  const ur = await load(bytes, {
    start,
    print: (kind, text) => printed[kind].push(text),
    download: (name, data) => files.push([name, data]),
  });
  return { ur, printed };
}

// ==============================================================================
// As the page uses it
// ==============================================================================

{
  const files = [];
  const { ur, printed } = await watch({ start: true, files });
  const log = printed[LOG];
  if (printed[ERROR].length) fail("booting printed errors", printed[ERROR]);

  // The start script set the time from the clock, and connected.
  ur.status();
  const status = printed[STATUS].at(-1);
  if (!/^\d\d:\d\d · /.test(status) || status.includes("no phone")) {
    fail(`after the start script, the status is ${JSON.stringify(status)}`);
  }

  // Expects the log to say `text` since `from` (an index into it).
  const expect = (from, text) => {
    if (!log.slice(from).some((line) => line.includes(text))) {
      fail(`expected ${JSON.stringify(text)} in the log`, log.slice(from));
    }
  };
  // Near the middle of the panel, in the drawn watch's pixels: a little
  // right of the watch's middle (the button sticks out on the left), and
  // above it (the strap is longer below). See src/case.rs.
  const middle = [ur.width * 0.52, ur.height * 0.41];

  let from = log.length;
  ur.pointer("down", ...middle);
  ur.advance(80);
  ur.pointer("up", ...middle);
  expect(from, "input: tap");

  from = log.length;
  ur.pointer("down", ...middle);
  ur.advance(700);
  expect(from, "input: long press");
  ur.pointer("up", ...middle);

  from = log.length;
  ur.pointer("down", middle[0], middle[1] + 60);
  for (let dy = 50; dy >= -60; dy -= 10) {
    ur.advance(10);
    ur.pointer("move", middle[0], middle[1] + dy);
  }
  ur.pointer("up", middle[0], middle[1] - 60);
  expect(from, "input: swipe up");

  // The button: the case's left edge, two thirds of the way down the panel.
  from = log.length;
  for (let x = 0; x < ur.width / 4 && log.length === from; x++) {
    ur.pointer("down", x, ur.height * 0.53);
    ur.pointer("up", x, ur.height * 0.53);
  }
  expect(from, "input: button");

  from = log.length;
  if (!ur.key("ArrowLeft")) fail("ArrowLeft isn't a key the watch takes");
  expect(from, "input: swipe left");
  if (ur.key("Shift")) fail("Shift is a key the watch takes");

  from = log.length;
  ur.key("n");
  expect(from, "input: notification");
  from = log.length;
  ur.key("h");
  ur.key("s");
  if (files.length !== 1 || files[0][0] !== "ur-sim-1.png") {
    fail(`S downloaded ${JSON.stringify(files.map(([name]) => name))}`);
  }
  // R records, and R again stops and downloads the GIF.
  ur.key("r");
  ur.key("ArrowRight");
  ur.advance(1000);
  ur.key("r");
  const gif = files[1];
  if (files.length !== 2 || gif[0] !== "ur-sim-1.gif" || new TextDecoder().decode(gif[1].slice(0, 6)) !== "GIF89a") {
    fail(`R, R downloaded ${JSON.stringify(files.map(([name]) => name))}`);
  }

  // Commands, good and bad.
  ur.command("echo hello");
  expect(from, "hello");
  ur.command("no-such-command");
  if (!printed[ERROR].some((line) => line.includes("unknown command"))) {
    fail("a bad command printed no error", printed[ERROR]);
  }

  ur.themes();
  // A line a scheme: its theme's name, a tab, its own.
  const themes = printed[THEMES].at(-1).split("\n").map((line) => line.split("\t")[1]);
  if (themes.length < 2) fail(`themes: ${JSON.stringify(themes)}`);
  printed[ERROR].length = 0;
  // Quoted, as a scheme's name may have a space.
  ur.command(`theme "${themes[1]}"`);
  if (printed[ERROR].length) fail(`theme "${themes[1]}"`, printed[ERROR]);

  // A frame is the whole watch, and the panel's lit: some of its pixels
  // are neither the desk, the case, nor the glass (all greys).
  ur.advance(1000);
  const frame = ur.frame();
  if (frame.length !== ur.width * ur.height * 4) fail(`a frame is ${frame.length} bytes`);
  let colours = 0;
  for (let i = 0; i < frame.length; i += 4) {
    if (frame[i] !== frame[i + 1] || frame[i + 1] !== frame[i + 2]) colours++;
  }
  if (colours === 0) fail("the frame has no colour: is the panel drawn?");
  // The real panel's colours, or as sent to it: the panel's pixels
  // differ, and each is the same every time. (With the screen on: off,
  // both are the glass.)
  ur.key("g");
  ur.advance(500);
  ur.mimic(false);
  const sent = ur.frame().slice();
  ur.mimic(true);
  const real = ur.frame().slice();
  if (sent.every((value, i) => value === real[i])) fail("ur.mimic changed nothing");
  ur.mimic(true);
  if (!ur.frame().every((value, i) => value === real[i])) fail("ur.mimic(true) drew another frame");
  console.log(`smoke: the page's watch works (${ur.width}×${ur.height}, ${themes.length} colour schemes)`);
}

// ==============================================================================
// Scripts, for comparing with the command line's screenshots
// ==============================================================================

for (const script of scripts) {
  const files = [];
  const { ur, printed } = await watch({ start: false, files });
  ur.script(readFileSync(script, "utf8"));
  if (printed[ERROR].length) fail(`${script} failed`, printed[ERROR]);
  const dir = join(out, basename(script, ".txt"));
  mkdirSync(dir, { recursive: true });
  for (const [name, data] of files) writeFileSync(join(dir, name), data);
  console.log(`smoke: ${script}: ${files.length} files in ${dir}`);
}
