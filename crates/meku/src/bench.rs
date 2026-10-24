//! Opt-in scroll benchmark (`MEKU_BENCH_SCROLL=<csv path>`): feeds synthetic
//! wheel notches through the real glide path, records every painted frame
//! (time, scroll offset) to a CSV, then quits. Dev tool; inert otherwise.

use std::cell::RefCell;
use std::io::Write as _;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::*;

use crate::smooth::{GlideState, Scroller};

/// (start ms, end ms, ms between notches) phases of the synthetic wheel.
const PHASES: &[(u128, u128, u128)] = &[(500, 3500, 80), (5000, 7500, 250)];
const END_MS: u128 = 9000;

pub(crate) fn path() -> Option<String> {
    std::env::var("MEKU_BENCH_SCROLL").ok()
}

struct Run {
    t0: Instant,
    last_notch: u128,
    rows: Vec<(f32, f32)>,
}

pub(crate) fn probe(state: &GlideState, scroller: Scroller, list: &ListState) -> impl IntoElement {
    thread_local! {
        static RUN: Rc<RefCell<Run>> = Rc::new(RefCell::new(Run {
            t0: Instant::now(),
            last_notch: 0,
            rows: Vec::new(),
        }));
    }
    let run = RUN.with(Rc::clone);
    let (state, list) = (state.clone(), list.clone());
    canvas(
        |_, _, _| {},
        move |_, _, window, cx| {
            let Some(out) = path() else { return };
            let ms = run.borrow().t0.elapsed().as_millis();
            let y = f32::from(list.scroll_px_offset_for_scrollbar().y);
            run.borrow_mut().rows.push((ms as f32, -y));
            if ms >= END_MS {
                let mut f = std::fs::File::create(&out).expect("bench csv");
                for (t, y) in &run.borrow().rows {
                    let _ = writeln!(f, "{t:.3},{y:.2}");
                }
                cx.quit();
                return;
            }
            let due = PHASES.iter().find(|(a, b, gap)| {
                ms >= *a && ms < *b && ms - run.borrow().last_notch >= *gap
            });
            if due.is_some() {
                run.borrow_mut().last_notch = ms;
                crate::smooth::notch(&state, &scroller, (0.0, -1.0), window, cx);
            }
            // Keep painting so idle frames are timed too.
            window.on_next_frame(|window, _| window.refresh());
        },
    )
    .absolute()
    .size_0()
}
