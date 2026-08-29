use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use yas::{log_error, log_info};

use super::capture_frame::CaptureFrame;
use super::grid_voter::GridAnnotation;

/// A work item sent from the capture thread to the worker pool.
pub struct WorkItem<M: Send> {
    pub index: usize,
    pub frame: CaptureFrame,
    pub metadata: M,
    /// Grid overlay annotation data for the page (for debug image dumps).
    pub grid_annotation: Option<GridAnnotation>,
}

/// Handle to a running worker. Call `join()` to wait for results.
pub struct WorkerHandle<R> {
    handle: std::thread::JoinHandle<(Vec<R>, Vec<usize>)>,
    /// Set to true by the worker when it detects problems (e.g., consecutive
    /// duplicates). The capture thread should check this periodically and stop.
    pub should_stop: Arc<AtomicBool>,
}

impl<R> WorkerHandle<R> {
    /// Wait for the worker to finish and return ordered results plus index map.
    ///
    /// Returns `(items, index_map)` where `index_map[i]` is the original
    /// work item index that produced `items[i]`. This allows correlating
    /// output positions with debug image folder names.
    pub fn join(self) -> (Vec<R>, Vec<usize>) {
        self.handle
            .join()
            .expect("工作线程崩溃 / Worker thread panicked")
    }

    /// Check if the worker has signaled that scanning should stop.
    pub fn stop_requested(&self) -> bool {
        self.should_stop.load(Ordering::Relaxed)
    }
}

enum WorkerEvent<R> {
    ItemProcessed(usize, anyhow::Result<Option<R>>),
    CaptureFinished { total: usize },
}

/// Announce the handoff from controller-driven capture to background OCR.
///
/// Call this only after the controller has submitted every screenshot for the
/// current phase. `processed` may be non-zero because OCR runs concurrently.
pub fn log_capture_finished(processed: usize, total: usize) {
    if processed < total {
        log_info!(
            "[OCR] 本轮截图采集已完成，OCR仍在处理中，请等待：{}/{}",
            "[OCR] this screenshot capture phase is complete; OCR is still processing, please wait: {}/{}",
            processed,
            total
        );
    } else {
        log_info!(
            "[OCR] 本轮截图采集已完成，OCR处理进度：{}/{}",
            "[OCR] this screenshot capture phase is complete; OCR processing progress: {}/{}",
            processed,
            total
        );
    }
}

/// Log OCR progress after [`log_capture_finished`] has announced the handoff.
pub fn log_ocr_progress(processed: usize, total: usize) {
    log_info!(
        "[OCR] 处理进度：{}/{}",
        "[OCR] processing progress: {}/{}",
        processed,
        total
    );
}

/// Start a parallel scan worker.
///
/// Items are received via the returned sender, dispatched to rayon for
/// parallel processing, and results collected in index order.
///
/// The `process_fn` receives a `WorkItem` and returns:
/// - `Ok(Some(result))` — include in output
/// - `Ok(None)` — skip this item (e.g., non-artifact)
/// - `Err(e)` — log error, skip item
///
/// OCR progress stays silent while the capture thread controls the game. Once
/// all screenshots have been submitted, the worker logs processed/total
/// progress and can signal the capture thread to stop via
/// `WorkerHandle::should_stop`.
pub fn start_worker<M, R, F>(
    process_fn: F,
) -> (crossbeam_channel::Sender<WorkItem<M>>, WorkerHandle<R>)
where
    M: Send + 'static,
    R: Send + 'static,
    F: Fn(WorkItem<M>) -> anyhow::Result<Option<R>> + Send + Sync + 'static,
{
    // Bounded channel prevents memory blowup if OCR falls behind capture.
    // Buffer of 16 items ≈ 16 × ~1MB = ~16MB max in-flight images.
    let (item_tx, item_rx) = crossbeam_channel::bounded::<WorkItem<M>>(16);
    let should_stop = Arc::new(AtomicBool::new(false));
    let should_stop_clone = should_stop.clone();

    let handle = std::thread::spawn(move || {
        let _native_crash_context = yas::native_crash::inherit_current_task();
        let process_fn = Arc::new(process_fn);

        // Result channel: rayon tasks send completed items here. The dispatch
        // thread sends CaptureFinished only after the screenshot sender closes,
        // so no OCR progress is logged while the controller is still active.
        let (result_tx, result_rx) = crossbeam_channel::unbounded::<WorkerEvent<R>>();

        // Dispatch: receive items and spawn rayon tasks
        let dispatch_result_tx = result_tx.clone();
        let dispatch_handle = std::thread::spawn(move || {
            let _native_crash_context = yas::native_crash::inherit_current_task();
            let mut total = 0;
            for item in item_rx {
                total += 1;
                let process_fn = process_fn.clone();
                let tx = dispatch_result_tx.clone();
                let index = item.index;
                rayon::spawn(move || {
                    let _native_crash_context = yas::native_crash::inherit_current_task();
                    let result = process_fn(item);
                    let _ = tx.send(WorkerEvent::ItemProcessed(index, result));
                });
            }
            let _ = dispatch_result_tx.send(WorkerEvent::CaptureFinished { total });
            // Drop our sender so result_rx eventually closes
            drop(dispatch_result_tx);
        });
        // Drop the original sender clone
        drop(result_tx);

        // Collection: reorder results via BTreeMap
        let mut results_map: BTreeMap<usize, anyhow::Result<Option<R>>> = BTreeMap::new();
        let mut next_index: usize = 0;
        let mut output: Vec<R> = Vec::new();
        let mut index_map: Vec<usize> = Vec::new();
        let mut consecutive_errors: usize = 0;
        let mut processed: usize = 0;
        let mut capture_total: Option<usize> = None;

        for event in result_rx {
            match event {
                WorkerEvent::ItemProcessed(index, result) => {
                    processed += 1;
                    results_map.insert(index, result);

                    if let Some(total) = capture_total {
                        log_ocr_progress(processed, total);
                    }

                    // Drain consecutive ready results
                    while let Some(result) = results_map.remove(&next_index) {
                        let current_index = next_index;
                        next_index += 1;

                        match result {
                            Ok(Some(item)) => {
                                output.push(item);
                                index_map.push(current_index);
                                consecutive_errors = 0;
                            },
                            Ok(None) => {
                                // Skipped item
                                consecutive_errors = 0;
                            },
                            Err(e) => {
                                log_error!(
                                    "[worker] 第{}项错误: {}",
                                    "[worker] item {} error: {}",
                                    current_index,
                                    e
                                );
                                consecutive_errors += 1;
                                if consecutive_errors >= 10 {
                                    log_error!(
                                        "[worker] 连续{}个错误，发送停止信号",
                                        "[worker] {} consecutive errors, signaling stop",
                                        consecutive_errors
                                    );
                                    should_stop_clone.store(true, Ordering::Relaxed);
                                }
                            },
                        }
                    }
                },
                WorkerEvent::CaptureFinished { total } => {
                    capture_total = Some(total);
                    log_capture_finished(processed, total);
                },
            }
        }

        // Drain any remaining buffered results
        while let Some(result) = results_map.remove(&next_index) {
            let current_index = next_index;
            next_index += 1;
            if let Ok(Some(item)) = result {
                output.push(item);
                index_map.push(current_index);
            }
        }

        let _ = dispatch_handle.join();

        (output, index_map)
    });

    (
        item_tx,
        WorkerHandle {
            handle,
            should_stop,
        },
    )
}
