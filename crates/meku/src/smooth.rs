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

use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::*;

/// Stiffness of the critically damped spring that carries the content to
/// where the wheel points (rad/s). It settles in about a quarter second and
/// never overshoots.
const OMEGA: f32 = 30.0;
/// Share of the exponential-approach speed a notch starts with. Zero starts
/// from rest (laggy), one starts at full speed (a visible kick).
const KICK: f32 = 0.5;
/// Distance one wheel notch asks for at the default speed, in line heights.
const NOTCH_LINES: f32 = 1.5;
/// Never queue more than this many pixels of unscrolled distance.
const MAX_PENDING: f32 = 600.0;
/// Assumed frame time for the first step after a notch, which lands before
/// any frame has timed it.
const FIRST_DT: f32 = 0.008;

/// What a smooth layer drives: move the content by `delta` (positive scrolls
/// toward the end) and report whether anything actually moved.
pub(crate) type Scroller = Rc<dyn Fn(Point<Pixels>, &mut App) -> bool>;

/// Distance still to travel and its rate of change per axis, the sub-pixel
/// remainder not yet applied, and whether a glide is running.
#[derive(Default)]
pub(crate) struct Glide {
    pending: Point<f32>,
    vel: Point<f32>,
    frac: Point<f32>,
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
                notch(&state, &scroller, (lines.x, lines.y), window, cx);
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Apply one wheel notch (in lines) to a glide: extend the distance to travel
/// and keep whatever speed the content already has, so back-to-back notches
/// flow into one motion.
pub(crate) fn notch(
    state: &GlideState,
    scroller: &Scroller,
    lines: (f32, f32),
    window: &mut Window,
    cx: &mut App,
) {
    let speed = crate::theme::cfg(cx).f64("scroll_speed") as f32;
    let dist = (f32::from(window.line_height()) * NOTCH_LINES * speed)
        .round()
        .max(1.0);
    let scale = window.scale_factor();
    let start = {
        let mut guard = state.borrow_mut();
        let g = &mut *guard;
        let (wx, wy) = (-lines.0 * dist, -lines.1 * dist);
        add(&mut g.pending.x, &mut g.vel.x, wx);
        add(&mut g.pending.y, &mut g.vel.y, wy);
        !g.running && (g.pending.x != 0.0 || g.pending.y != 0.0)
    };
    if !start {
        return;
    }
    // Show the first movement in the very next frame instead of one later.
    let d = {
        let mut g = state.borrow_mut();
        g.running = true;
        g.last = Some(Instant::now());
        advance(&mut g, FIRST_DT, scale).0
    };
    scroller(point(px(d.x), px(d.y)), cx);
    window.refresh();
    step(state.clone(), scroller.clone(), window);
}

/// Add `w` pixels of travel to one axis. A reversal drops the old glide so the
/// content turns around at once. Starting from rest, the notch also gives the
/// spring a head start toward the target; mid-flight the content already has
/// speed, which is kept as it is.
fn add(pending: &mut f32, vel: &mut f32, w: f32) {
    if w * *pending < 0.0 {
        *pending = 0.0;
        *vel = 0.0;
    }
    if *pending == 0.0 {
        *vel = KICK * OMEGA * w;
    }
    *pending = (*pending + w).clamp(-MAX_PENDING, MAX_PENDING);
}

/// A vertical scroll area with a scroll bar and smooth wheel, built the way
/// the kit lays them out: `outer` (sized by the caller) holds the scrolling
/// `inner` area, and the wheel layer and the bar are its siblings, so neither
/// scrolls with, or adds to, the content.
pub(crate) fn frame<E>(
    id: &'static str,
    outer: Div,
    inner: E,
    handle: &ScrollHandle,
    glide: &GlideState,
) -> Div
where
    E: Styled + StatefulInteractiveElement + IntoElement,
{
    outer
        .relative()
        .child(inner.size_full().overflow_y_scroll().track_scroll(handle))
        .child(wheel(glide, for_handle(handle, false)))
        .child(
            div()
                .absolute()
                .inset_0()
                .child(Scrollbar::vertical(handle).id(id).viewport_from_layout()),
        )
}

/// Advance one axis by `dt` seconds along the exact solution of a critically
/// damped spring (`e` is the distance left, `u` the speed toward the target).
/// Returns the whole device pixels to scroll now; the sub-pixel rest stays in
/// `frac` so no distance is lost and text never sits between pixels.
fn axis(e: &mut f32, u: &mut f32, frac: &mut f32, dt: f32, scale: f32) -> f32 {
    let k = (-OMEGA * dt).exp();
    let c = OMEGA * *e - *u;
    let (ne, nu) = ((*e + c * dt) * k, (*u + OMEGA * c * dt) * k);
    let mut moved = *e - ne;
    (*e, *u) = (ne, nu);
    let settled = ne.abs() * scale < 0.5 && nu.abs() * scale < 4.0;
    if settled {
        moved += ne;
        (*e, *u) = (0.0, 0.0);
    }
    let total = moved + *frac;
    let applied = (total * scale).round() / scale;
    *frac = if settled { 0.0 } else { total - applied };
    applied
}

/// Move the glide forward by `dt` seconds: returns the pixels to scroll and
/// whether it has come to rest.
fn advance(g: &mut Glide, dt: f32, scale: f32) -> (Point<f32>, bool) {
    let x = axis(&mut g.pending.x, &mut g.vel.x, &mut g.frac.x, dt, scale);
    let y = axis(&mut g.pending.y, &mut g.vel.y, &mut g.frac.y, dt, scale);
    let done = g.pending.x == 0.0 && g.pending.y == 0.0;
    (point(x, y), done)
}

/// One animation frame: move along the spring by the time since the last one,
/// then ask for the next frame until it has come to rest.
fn step(state: GlideState, scroller: Scroller, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        let scale = window.scale_factor();
        let now = Instant::now();
        let (delta, done) = {
            let mut g = state.borrow_mut();
            let dt = g
                .last
                .map_or(0.016, |t| now.duration_since(t).as_secs_f32())
                .clamp(0.001, 0.05);
            g.last = Some(now);
            advance(&mut g, dt, scale)
        };
        let moved = scroller(point(px(delta.x), px(delta.y)), cx);
        window.refresh();
        if done || !moved {
            let mut g = state.borrow_mut();
            g.running = false;
            if !moved {
                *g = Glide::default(); // hit the end: drop the rest
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
    use super::{Glide, add, advance};

    const DT: f32 = 1.0 / 120.0;

    fn glide(dist: f32) -> Glide {
        let mut g = Glide::default();
        add(&mut g.pending.y, &mut g.vel.y, dist);
        g
    }

    /// Per-frame steps until rest.
    fn run(g: &mut Glide, dt: f32, scale: f32) -> Vec<f32> {
        let mut steps = Vec::new();
        for _ in 0..2000 {
            let (d, done) = advance(g, dt, scale);
            steps.push(d.y);
            if done {
                break;
            }
        }
        steps
    }

    #[test]
    fn delivers_the_whole_distance_without_overshoot() {
        for scale in [1.0, 1.5, 2.0] {
            for dt in [1.0 / 60.0, DT, 1.0 / 144.0] {
                let mut g = glide(36.0);
                let steps = run(&mut g, dt, scale);
                let mut pos = 0.0;
                for d in &steps {
                    pos += d;
                    assert!(pos <= 36.0 + 0.5 / scale, "overshoot {pos} at {scale}x");
                }
                assert!((pos - 36.0).abs() <= 0.5 / scale, "delivered {pos}");
                assert!(steps.len() > 8, "spread over frames: {}", steps.len());
            }
        }
    }

    #[test]
    fn steps_rise_and_fall_without_a_spike() {
        let mut g = glide(36.0);
        let steps = run(&mut g, DT, 1.0);
        let peak = steps.iter().cloned().fold(0.0, f32::max);
        assert!(peak < 36.0 * 0.2, "no single-frame jump: {peak}");
        assert!(steps[0] > 0.0, "moves in the very first frame");
    }

    #[test]
    fn a_second_notch_mid_flight_keeps_speed_continuous() {
        let mut g = glide(36.0);
        let mut prev = 0.0;
        for _ in 0..6 {
            prev = advance(&mut g, DT, 1.0).0.y;
        }
        add(&mut g.pending.y, &mut g.vel.y, 36.0);
        let next = advance(&mut g, DT, 1.0).0.y;
        assert!(
            (next - prev).abs() < 2.5,
            "per-frame step jumps from {prev} to {next}"
        );
    }

    #[test]
    fn reversing_turns_around_at_once() {
        let mut g = glide(36.0);
        for _ in 0..6 {
            advance(&mut g, DT, 1.0);
        }
        add(&mut g.pending.y, &mut g.vel.y, -36.0);
        assert!(advance(&mut g, DT, 1.0).0.y < 0.0);
    }

    #[test]
    fn a_stalled_frame_cannot_overshoot() {
        let mut g = glide(100.0);
        let (d, _) = advance(&mut g, 0.05, 1.0);
        assert!(d.y > 0.0 && d.y <= 100.0, "{}", d.y);
    }
}
