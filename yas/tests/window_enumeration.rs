//! Regression test for the `iterate_window()` data race.
//!
//! `iterate_window` used to accumulate results into a process-wide
//! `static mut Vec<HWND>` mutated from an `extern "system"` EnumWindows
//! callback with no synchronisation, while being called concurrently from
//! the HTTP thread (`GET /health`) and the scanner execution thread.
//!
//! Before the fix this test corrupted the process heap and aborted with
//! STATUS_HEAP_CORRUPTION (0xC0000374) within a few hundred iterations.
//! After the fix each call owns its own buffer, so concurrent callers see
//! self-consistent results.

#![cfg(target_os = "windows")]

use std::collections::BTreeSet;

#[test]
fn iterate_window_is_safe_under_concurrency() {
    const THREADS: usize = 8;
    const ITERS: usize = 2000;

    let workers: Vec<_> = (0..THREADS)
        .map(|_| {
            std::thread::spawn(|| {
                let mut lengths = BTreeSet::new();
                let mut null_handles = 0usize;

                for _ in 0..ITERS {
                    let handles = yas_core::utils::iterate_window();

                    // Touch every element. A buffer that was freed or aliased
                    // by another thread shows up as a null/garbage HWND.
                    for h in &handles {
                        if h.is_null() {
                            null_handles += 1;
                        }
                    }
                    lengths.insert(handles.len());
                }

                (lengths, null_handles)
            })
        })
        .collect();

    let mut total_nulls = 0usize;
    let mut all_lengths = BTreeSet::new();
    for w in workers {
        let (lengths, nulls) = w.join().expect("worker thread aborted");
        all_lengths.extend(lengths);
        total_nulls += nulls;
    }

    assert_eq!(
        total_nulls, 0,
        "EnumWindows never yields a null HWND; {total_nulls} null handles means \
         the result buffer was concurrently reallocated"
    );

    // The real window count drifts as other processes open and close windows,
    // so we can't demand a single exact length. We can demand it stays in a
    // plausible band — a torn (ptr, len, cap) triple produces wild outliers.
    let min = *all_lengths.iter().next().expect("no results collected");
    let max = *all_lengths.iter().next_back().unwrap();
    assert!(min > 0, "EnumWindows returned no windows at all");
    assert!(
        max - min < min / 2 + 64,
        "window count swung wildly across concurrent calls (min={min}, max={max}), \
         which indicates the shared accumulator was torn by a data race"
    );
}
