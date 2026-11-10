/* For the `coverage` feature only (see src/coverage.rs): where
 * SanitizerCoverage's sections go. build.rs passes this to the linker with
 * the feature, and never without it.
 *
 * - Each function's flag (a byte; `sim/coverage-build.sh` instruments
 *   function entries, as branches don't fit): in RAM, after `.uninit`. A
 *   `compare` build leaves only ~300 bytes free there, so the coverage
 *   build shrinks defmt-rtt's buffer (`.uninit`, 1 KiB) to 256 bytes,
 *   which makes room. (Not the stack's spare room: a compare build's stack
 *   is already close to its end on the deepest paths.) cortex-m-rt
 *   neither zeroes nor paints them there; `__pre_init` (src/coverage.rs)
 *   zeroes them.
 * - The PC table (for each flag, its function's address): in the ELF only,
 *   as a non-allocated section, so it costs no flash. The linker still
 *   resolves its addresses.
 *
 * LLVM's constructor refers to both by their `__start_`/`__stop_` symbols,
 * so they're defined here too; the simulator uses `__sancov_bools_start`
 * and `__sancov_bools_end`. */
SECTIONS
{
  .sancov_bools (NOLOAD) : ALIGN(4)
  {
    __sancov_bools_start = .;
    __start___sancov_bools = .;
    KEEP(*(__sancov_bools));
    __stop___sancov_bools = .;
    __sancov_bools_end = .;
  } > RAM

  .sancov_pcs (INFO) :
  {
    __start___sancov_pcs = .;
    KEEP(*(__sancov_pcs));
    __stop___sancov_pcs = .;
  }
} INSERT AFTER .uninit;
