# nrf-mpsl: please release the timeslot-lock fix (#124)

File at: <https://github.com/alexmoon/nrf-sdc/issues/new>

**Title:** `nrf-mpsl`: release with the RTC0 timeslot-lock fix (#124)?

nrf-mpsl 0.4.0 (the latest on crates.io, 2026-08-21) masks only TIMER0 in
`Flash`'s state lock, but MPSL also delivers `MPSL_TIMESLOT_SIGNAL_START` from
RTC0's interrupt. If that lands while thread mode holds the lock, the
callback's `RefCell::borrow_mut` panics:

    timeslot_session_callback <- MPSL_IRQ_RTC0_Handler <- with_inner (do_op's poll)

On an nRF52832 saving back to back, this reset the chip within ~70 saves.
#124 (a0eafe0, 383afed) fixes it on `main`. Could it go into a 0.4.x release?
