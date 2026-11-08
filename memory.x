/* nRF52832-QFAA as used in the PineTime: 512 KiB flash, 64 KiB RAM.
 *
 * We run under the PineTime's stock bootloader (MCUBoot 1.5, as shipped with
 * InfiniTime), so that sealed watches can be updated over Bluetooth and can
 * always roll back. Its internal flash layout, which we must not disturb:
 *
 *   0x00000  bootloader (28 KiB)
 *   0x07000  bootloader's page: reboot log, and at 0x7F00 a copy of our
 *            vector table that it writes on every boot (VTOR points there)
 *   0x08000  primary image slot (464 KiB), holding:
 *              0x08000  MCUBoot image header, padded to 256 bytes
 *              0x08100  our vector table and code (FLASH below)
 *                       ... MCUBoot's TLVs (the image hash) after the code
 *              0x7BE50  the slot's trailer: swap state and the "image OK"
 *                       flag that `firmware::confirm` sets
 *   0x7C000  MCUBoot's swap scratch area (4 KiB)
 *   0x7D000  unused by the bootloader: our persistent storage (12 KiB)
 *
 * The header is padded to 256 bytes (InfiniTime uses 32) because the
 * Cortex-M4 requires the vector table to be aligned to its size rounded up to
 * a power of two (256 bytes here), and cortex-m-rt checks this. The bootloader
 * reads the header size from the header, so any size works for it.
 *
 * `scripts/package.py` wraps the linked firmware in the header and TLVs, and
 * its constants must match these.
 */
MEMORY
{
  /* Slot end, less header, trailer (0x1B0), and TLVs (0x28: SHA-256). */
  FLASH    : ORIGIN = 0x00008100, LENGTH = 0x7C000 - 0x8100 - 0x1B0 - 0x28
  STORAGE  : ORIGIN = 0x0007D000, LENGTH = 12K
  /* Our statics. The stack doesn't live here; see `_stack_start`. */
  RAM      : ORIGIN = 0x20000000, LENGTH = 0xD400
  /* State that survives resets, at fixed addresses so it survives resets
   * into *different* firmware (reflashing, updates), whose RAM layout may
   * differ: today's steps (src/activity.rs) at +0, the wall-clock checkpoint
   * (src/clock.rs) at +0x20, where the last panic (or watchdog bite)
   * happened, with its message and a backtrace (src/field_log.rs), at +0x40
   * to the end.
   *
   * Every reset runs the bootloader first, and it uses far more RAM than
   * its symbols suggest. Painting RAM over SWD, then running the bootloader
   * to our entry point (with an earlier update in the secondary slot, the
   * usual state) showed it writing everything up to 0x2000CB80 (statics and
   * heap) and down to 0x2000DD60 (stack). The retained block sits in the
   * middle of that gap. Other paths (a swap, a rollback) may reach further;
   * then the magic numbers reject what's left, and the state is lost, not
   * misread. */
  RETAINED : ORIGIN = 0x2000D400, LENGTH = 256
}

/* The stack takes the 11 KiB above the retained block, growing down towards
 * it. Embassy tasks keep their state in statics, not on the stack, but a
 * task's poll still nests every call it makes. The deepest are the
 * Bluetooth host's elliptic-curve maths, when pairing (7.7 KB measured,
 * radio interrupts included) and when booting (7.6 KB; see `ble::build`).
 * The hourly power log reports each hour's deepest (`power`). Our statics have only 53 KiB
 * (`RAM`), so the retained block stays clear of the bootloader.
 *
 * `_stack_end` bounds what cortex-m-rt paints at boot (`paint-stack`), for
 * that report: the retained block must not be painted over. It must match
 * `field_log::STACK_END`. */
_stack_start = 0x20010000;
_stack_end = ORIGIN(RETAINED) + LENGTH(RETAINED);
