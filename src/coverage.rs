//! Edge coverage, for the dev kit fuzzer (`ur-sim --fuzz`): with the
//! `coverage` feature, and built by `sim/coverage-build.sh`, which has LLVM's
//! SanitizerCoverage give each function a byte that its entry sets to 1.
//! Every crate's, embassy's and trouble's too, after inlining: branches
//! (~11,000 of them) would take ~150 KB more flash and ~11 KB of RAM, and
//! LLVM's allowlist, which could have kept the instrumentation to this
//! crate, is clang's alone. The flags take RAM from defmt-rtt's buffer
//! (see `coverage.x`). The simulator reads them over the probe
//! (`esprobe fast-dump`, which doesn't halt the core) after each step, and
//! maps each byte to its branch through the PC table, which `coverage.x`
//! keeps in the ELF but not in flash.
//!
//! Not for releases: `scripts/release.sh` never sets the feature, and
//! without it this module isn't built and `build.rs` adds no linker script,
//! so a release image is byte for byte what it was.

// LLVM calls these from a constructor, to hand a runtime the flags and the
// PC table. cortex-m-rt runs no constructors, but the call still has to
// link; the simulator finds both through the ELF's symbols instead.

/// SanitizerCoverage's hook for the flags (`-sanitizer-coverage-inline-
/// bool-flag`): nothing to do.
#[unsafe(no_mangle)]
extern "C" fn __sanitizer_cov_bool_flag_init(_start: *const u8, _end: *const u8) {}

/// SanitizerCoverage's hook for the PC table (`-sanitizer-coverage-pc-
/// table`): nothing to do.
#[unsafe(no_mangle)]
extern "C" fn __sanitizer_cov_pcs_init(_start: *const usize, _end: *const usize) {}

// The flags live after `.uninit` (see `coverage.x`), which cortex-m-rt
// neither zeroes nor paints, and the bootloader leaves RAM full of its own
// data. So clear them before anything runs: in `__pre_init`, which
// cortex-m-rt calls before it initializes RAM, and which (its docs say) is
// best written in assembly, as no Rust may run before RAM is initialized.
core::arch::global_asm!(
    ".section .text.__pre_init,\"ax\",%progbits",
    ".global __pre_init",
    ".type __pre_init,%function",
    ".thumb_func",
    "__pre_init:",
    "    ldr r0, =__sancov_bools_start",
    "    ldr r1, =__sancov_bools_end",
    "    movs r2, #0",
    "1:  cmp r0, r1",
    "    bhs 2f",
    "    strb r2, [r0], #1",
    "    b 1b",
    "2:  bx lr",
    ".ltorg",
);
