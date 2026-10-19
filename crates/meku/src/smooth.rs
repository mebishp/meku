//! Smooth mouse-wheel scrolling.
//!
//! A wheel notch arrives as a whole-line jump, which reads as stepping.
//! [`wheel`] adds an invisible layer over a scroll container that catches
//! those notch events and eases the container toward where they point
//! instead (exponential approach, frame-rate independent), so rapid notches
//! accumulate into one glide. Touchpads send exact pixel deltas, already
//! smooth, so they pass through to the container untouched.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::*;

/// Time constant of the glide: short, so it settles in about 100ms.
/// Half of every notch is applied the moment it arrives, so the response
/// is immediate and only the tail eases.
const TAU_SECS: f32 = 0.03;
/// Distance one wheel notch asks for, in line heights.
const NOTCH_LINES: f32 = 3.0;
/// Never queue more than this many pixels of unscrolled distance.
const MAX_PENDING: f32 = 600.0;
/// Share of a notch applied instantly.
const INSTANT: f32 = 0.5;

/// What a smooth layer drives: move the content by `delta` (positive scrolls
/// toward the end) and report whether anything actually moved.
pub(crate) type Scroller = Rc<dyn Fn(Point<Pixels>, &mut App) -> bool>;

/// Scroll position unaccounted for yet, and whether a glide is running.
#[derive(Default)]
pub(crate) struct Glide {
    pending: Point<f32>,
    running: bool,
    last: Option<Instant>,
}

pub(crate) type GlideState = Rc<RefCell<Glide>>;

/// Scroller for a virtual list (vertical only).
pub(crate) fn for_list(list: &ListState) -> Scroller {
    let list = list.clone();
    Rc::new(move |delta, _| {
        list.scroll_by(delta.y);
        true
    })
}

/// Scroller for a tracked scroll container. `vertical_to_horizontal` makes
/// the wheel's vertical motion scroll a horizontal strip (the tab bar).
pub(crate) fn for_handle(handle: &ScrollHandle, vertical_to_horizontal: bool) -> Scroller {
    let handle = handle.clone();
    Rc::new(move |delta, _| {
        let max = handle.max_offset();
        let old = handle.offset();
        let (dx, dy) = if vertical_to_horizontal {
            (delta.y, px(0.))
        } else {
            (delta.x, delta.y)
        };
        let new = point(
            (old.x - dx).clamp(-max.x, px(0.)),
            (old.y - dy).clamp(-max.y, px(0.)),
        );
        handle.set_offset(new);
        new != old
    })
}

/// An invisible layer to place as the last child of a scroll container (which
/// must be `relative`/positioned). It takes mouse-wheel notches over its
/// bounds and glides `scroller` toward them.
pub(crate) fn wheel(state: &GlideState, scroller: Scroller) -> impl IntoElement {
    let state = state.clone();
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                    return;
                }
                let ScrollDelta::Lines(lines) = event.delta else {
                    return; // touchpad: exact pixels, leave to the container
                };
                cx.stop_propagation();
                // Whole pixels only: fractional offsets shimmer the text.
                let notch = (f32::from(window.line_height()) * NOTCH_LINES).round();
                let instant = {
                    let mut g = state.borrow_mut();
                    // Reversing direction cancels what is left of the old
                    // glide, so the content turns around at once.
                    let (wx, wy) = (-lines.x * notch, -lines.y * notch);
                    if wx * g.pending.x < 0.0 {
                        g.pending.x = 0.0;
                    }
                    if wy * g.pending.y < 0.0 {
                        g.pending.y = 0.0;
                    }
                    let now = point((wx * INSTANT).round(), (wy * INSTANT).round());
                    g.pending.x = (g.pending.x + wx - now.x).clamp(-MAX_PENDING, MAX_PENDING);
                    g.pending.y = (g.pending.y + wy - now.y).clamp(-MAX_PENDING, MAX_PENDING);
                    now
                };
                scroller(point(px(instant.x), px(instant.y)), cx);
                window.refresh();
                {
                    let mut g = state.borrow_mut();
                    if g.running || (g.pending.x == 0.0 && g.pending.y == 0.0) {
                        return;
                    }
                    g.running = true;
                    g.last = Some(Instant::now());
                }
                step(state.clone(), scroller.clone(), window);
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Consume one frame's share of the pending distance at time `now`:
/// returns the pixels to scroll and whether the glide is finished.
fn advance(g: &mut Glide, now: Instant) -> (Point<f32>, bool) {
    let dt = g
        .last
        .map_or(0.016, |t| now.duration_since(t).as_secs_f32())
        .clamp(0.001, 0.05);
    g.last = Some(now);
    let k = 1.0 - (-dt / TAU_SECS).exp();
    // Whole pixels, at least one while anything remains, never past the end.
    let part = |p: f32| {
        if p.abs() < 1.0 {
            return p;
        }
        let step = (p * k).round();
        if step == 0.0 { p.signum() } else { step }
    };
    let d = point(part(g.pending.x), part(g.pending.y));
    g.pending.x -= d.x;
    g.pending.y -= d.y;
    (d, g.pending.x.abs() < 0.5 && g.pending.y.abs() < 0.5)
}

/// One animation frame: move a fraction of the remaining distance, then ask
/// for the next frame until it is used up.
fn step(state: GlideState, scroller: Scroller, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        let now = Instant::now();
        let (delta, done) = advance(&mut state.borrow_mut(), now);
        let moved = scroller(point(px(delta.x), px(delta.y)), cx);
        window.refresh();
        if done || !moved {
            let mut g = state.borrow_mut();
            g.running = false;
            if !moved {
                g.pending = point(0.0, 0.0); // hit the end: drop the rest
            }
            return;
        }
        step(state.clone(), scroller.clone(), window);
    });
}

/// Glide state and scroll handles for every scroll container in the shell.
#[derive(Default)]
pub(crate) struct Smooth {
    pub doc: GlideState,
    pub tree: GlideState,
    pub outline: GlideState,
    pub tabs: GlideState,
    pub settings: GlideState,
    pub palette: GlideState,
    pub lightbox: GlideState,
    pub tree_h: ScrollHandle,
    pub outline_h: ScrollHandle,
    pub settings_h: ScrollHandle,
    pub palette_h: ScrollHandle,
    pub lightbox_h: ScrollHandle,
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Glide, advance};
    use gpui_kit::point;

    #[test]
    fn glide_covers_the_distance_in_decaying_steps() {
        let mut g = Glide {
            pending: point(0.0, 60.0),
            running: true,
            last: Some(Instant::now()),
        };
        let mut now = Instant::now();
        let (mut total, mut steps, mut prev) = (0.0, 0, f32::MAX);
        loop {
            now += Duration::from_millis(16);
            let (d, done) = advance(&mut g, now);
            if !done {
                assert!(d.y <= prev + 1.0, "steps shrink as the glide settles");
                assert_eq!(d.y, d.y.round(), "whole pixels only");
            }
            prev = d.y;
            total += d.y;
            steps += 1;
            if done || steps > 200 {
                break;
            }
        }
        assert!(
            (total - 60.0).abs() < 0.01,
            "all of it is delivered: {total}"
        );
        assert!(
            (2..40).contains(&steps),
            "spread over several frames, not one: {steps}"
        );
    }

    #[test]
    fn a_stalled_frame_cannot_overshoot() {
        let mut g = Glide {
            pending: point(0.0, 100.0),
            running: true,
            last: Some(Instant::now()),
        };
        let (d, _) = advance(&mut g, Instant::now() + Duration::from_secs(5));
        assert!(
            d.y > 0.0 && d.y < 100.0,
            "a long pause still eases: {}",
            d.y
        );
    }
}
