//! Press-and-hold auto-repeat for stepper buttons.
//!
//! Clicking a "next / previous" arrow once per step is fine for a nudge and
//! useless for browsing 1440 sectors. Holding one down fires immediately, waits
//! out a short delay so a single click stays a single step, then repeats at a
//! rate that ramps up the longer it is held.
//!
//! The timing is a pure function of the frame clock ([`HoldRepeat::steps`]), so
//! it is unit-testable without a UI; the egui glue is [`repeat_button`].

/// How long a button must be held before it starts repeating. Long enough that
/// an ordinary click never repeats, short enough not to feel stuck.
const DELAY: f64 = 0.35;

/// Steps per second when repeating starts, and after the ramp has finished.
const MIN_RATE: f64 = 20.0;
const MAX_RATE: f64 = 150.0;

/// Seconds of holding it takes to accelerate from [`MIN_RATE`] to [`MAX_RATE`].
const RAMP: f64 = 2.0;

/// Steps per second for a button held for `held` seconds. Zero during the
/// initial delay, then a linear ramp from [`MIN_RATE`] to [`MAX_RATE`].
fn rate(held: f64) -> f64 {
    if held < DELAY {
        return 0.0;
    }
    let t = ((held - DELAY) / RAMP).clamp(0.0, 1.0);
    MIN_RATE + (MAX_RATE - MIN_RATE) * t
}

/// One button being held down.
struct Hold {
    key: &'static str,
    /// Frame time the press began.
    started: f64,
    /// Frame time the last step fired.
    last_step: f64,
    /// Whether the immediate step owed to the press has been delivered.
    primed: bool,
}

/// Auto-repeat state shared by a group of stepper buttons. Only one button can
/// be held at a time, so a single instance serves a whole group; `key`
/// identifies which one.
#[derive(Default)]
pub struct HoldRepeat {
    active: Option<Hold>,
}

impl HoldRepeat {
    /// Begin holding `key`, replacing any hold in progress. The first
    /// [`steps`](Self::steps) call after this returns one step, so a plain
    /// click acts immediately.
    pub fn press(&mut self, key: &'static str, now: f64) {
        self.active = Some(Hold {
            key,
            started: now,
            last_step: now,
            primed: false,
        });
    }

    /// Whether `key` is the button currently being held.
    pub fn is_held(&self, key: &'static str) -> bool {
        self.active.as_ref().is_some_and(|h| h.key == key)
    }

    /// Note that `key` is no longer held, so the next press starts over.
    pub fn release(&mut self, key: &'static str) {
        if self.is_held(key) {
            self.active = None;
        }
    }

    /// How many steps `key` should fire on a frame at time `now`, given that it
    /// is still held down.
    ///
    /// Steps are derived from elapsed time rather than counted per frame, so
    /// the repeat speed does not depend on the frame rate and a slow frame
    /// catches up instead of dropping the steps it missed.
    pub fn steps(&mut self, key: &'static str, now: f64) -> usize {
        let Some(hold) = self.active.as_mut().filter(|h| h.key == key) else {
            return 0;
        };
        if !hold.primed {
            hold.primed = true;
            hold.last_step = now;
            return 1;
        }
        let rate = rate(now - hold.started);
        if rate <= 0.0 {
            // Still inside the delay; keep the repeat clock at "now" so the
            // first repeat lands one interval after the delay expires.
            hold.last_step = now;
            return 0;
        }
        let steps = ((now - hold.last_step) * rate).floor();
        if steps < 1.0 {
            return 0;
        }
        // Advance by exactly the steps taken, not to `now`, so fractional time
        // carries over instead of being dropped every frame.
        hold.last_step += steps / rate;
        steps as usize
    }
}

/// A stepper button that fires once when pressed and keeps firing while held.
/// Returns the number of steps to apply this frame (0 when idle).
///
/// `key` must be unique within the `hold` group and stable across frames.
pub fn repeat_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    hold: &mut HoldRepeat,
    key: &'static str,
) -> usize {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    let (now, primary_down) = ui.input(|i| (i.time, i.pointer.primary_down()));

    // Start from egui's own press detection, which resolves the press against
    // this button reliably at the moment it happens...
    if enabled && response.is_pointer_button_down_on() && !hold.is_held(key) {
        hold.press(key, now);
    }
    // ...but keep the hold alive from the raw pointer button. egui drops a
    // press from its click tracking once it can no longer become a click —
    // after `max_click_duration` (0.8 s by default) or a 6 px pointer wobble —
    // so `is_pointer_button_down_on` goes false part way through a hold. Left
    // to decide the hold, it ended the repeat after about a second, or the
    // moment the mouse drifted.
    if !enabled || !primary_down || !hold.is_held(key) {
        hold.release(key);
        return 0;
    }
    // egui sleeps between input events, and a held button produces none. Keep
    // frames coming so the repeat has a clock to run on.
    ui.ctx().request_repaint();
    hold.steps(key, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a hold of `key` from `press` time 0 to `seconds`, at 60 fps,
    /// returning the total steps fired.
    fn hold_for(seconds: f64) -> usize {
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        let mut total = hold.steps("next", 0.0);
        let mut t = 0.0;
        while t < seconds {
            t += 1.0 / 60.0;
            total += hold.steps("next", t);
        }
        total
    }

    #[test]
    fn a_press_fires_one_step_immediately() {
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        assert_eq!(hold.steps("next", 0.0), 1);
    }

    #[test]
    fn steps_without_a_press_do_nothing() {
        let mut hold = HoldRepeat::default();
        assert_eq!(hold.steps("next", 0.0), 0);
    }

    #[test]
    fn holding_does_not_repeat_during_the_delay() {
        // A single click, held for just under the delay, must stay one step.
        assert_eq!(hold_for(DELAY - 0.02), 1);
    }

    #[test]
    fn repeating_starts_after_the_delay_and_accelerates() {
        // Half a second in, only the delay plus a few repeats have elapsed.
        let early = hold_for(0.5);
        assert!((3..=6).contains(&early), "half a second gave {early} steps");
        // Held for four seconds it has ramped to the top rate, so the count is
        // far beyond what a fixed slow rate would give.
        let late = hold_for(4.0);
        assert!(late > 400, "four seconds gave {late} steps");
    }

    /// Regression: the repeat used to be driven by egui's
    /// `is_pointer_button_down_on`, which egui clears once a press exceeds
    /// `max_click_duration` (0.8 s). That capped every hold at ~17 steps and
    /// forced the user to keep re-pressing. Nothing in the timing may impose
    /// its own ceiling: a long hold must keep stepping.
    #[test]
    fn a_long_hold_keeps_stepping_past_the_click_window() {
        // egui's default `max_click_duration`, the point the old code died at.
        const CLICK_WINDOW: f64 = 0.8;
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        hold.steps("next", 0.0);

        let mut after_window = 0;
        let mut t = 0.0;
        while t < 6.0 {
            t += 1.0 / 60.0;
            let steps = hold.steps("next", t);
            if t > CLICK_WINDOW {
                after_window += steps;
            }
        }
        // Six seconds of holding at the top rate is hundreds of sectors, far
        // more than the ~17 the old behaviour managed in total.
        assert!(
            after_window > 600,
            "expected the hold to keep going past {CLICK_WINDOW}s, got {after_window} steps"
        );
        // And it is still going at the end, not silently exhausted.
        assert!(hold.steps("next", t + 1.0 / 60.0) > 0);
    }

    /// Steps come from elapsed time, so a slow frame catches up rather than
    /// dropping the steps it missed.
    #[test]
    fn a_slow_frame_catches_up() {
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        hold.steps("next", 0.0);
        hold.steps("next", DELAY); // delay expires
        let after_a_long_frame = hold.steps("next", DELAY + 0.5);
        assert!(
            after_a_long_frame >= 10,
            "expected a catch-up burst, got {after_a_long_frame}"
        );
    }

    #[test]
    fn only_the_held_button_steps() {
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        assert!(hold.is_held("next"));
        assert!(!hold.is_held("prev"));
        // The other arrow is polled every frame too; it must stay silent.
        assert_eq!(hold.steps("prev", 2.0), 0);
    }

    #[test]
    fn pressing_the_other_button_restarts_the_hold() {
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        hold.steps("next", 0.0);
        assert!(hold.steps("next", 2.0) > 1); // repeating fast by now
        hold.press("prev", 2.0);
        // The new hold acts once, then waits out its own delay rather than
        // inheriting the previous button's speed.
        assert_eq!(hold.steps("prev", 2.0), 1);
        assert_eq!(hold.steps("prev", 2.0 + 1.0 / 60.0), 0);
    }

    #[test]
    fn release_lets_the_next_press_fire_again() {
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        assert_eq!(hold.steps("next", 0.0), 1);
        assert_eq!(hold.steps("next", 0.1), 0); // still inside the delay
        hold.release("next");
        assert_eq!(hold.steps("next", 0.15), 0); // released: nothing to step
        hold.press("next", 0.2);
        assert_eq!(hold.steps("next", 0.2), 1);
    }

    #[test]
    fn releasing_another_button_leaves_the_active_hold_alone() {
        let mut hold = HoldRepeat::default();
        hold.press("next", 0.0);
        hold.steps("next", 0.0);
        hold.release("prev");
        assert!(hold.is_held("next"));
        assert_eq!(hold.steps("next", 0.1), 0); // inside the delay, still held
    }

    /// Drive [`repeat_button`] in a real egui context with the primary mouse
    /// button pressed and held for `seconds`, and count the steps produced.
    /// Frames run at 60 fps and no further pointer events are sent, so the
    /// button stays down exactly as it would under a real finger.
    fn steps_while_held_in_egui(seconds: f64) -> usize {
        let ctx = egui::Context::default();
        let mut hold = HoldRepeat::default();
        let mut total = 0usize;

        let mut frame = |time: f64, events: Vec<egui::Event>, total: &mut usize| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut rect = egui::Rect::ZERO;
            let _ = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    // Wrapped in a scope so `rect` is the button's own bounds:
                    // the panel's own `min_rect` is stretched to the full
                    // panel width, whose centre is nowhere near the button.
                    let scope =
                        ui.scope(|ui| repeat_button(ui, "\u{25B6}", true, &mut hold, "next"));
                    *total += scope.inner;
                    rect = scope.response.rect;
                });
            });
            rect
        };

        // Lay the button out to learn where it is, then hover it: egui
        // hit-tests the pointer against the *previous* frame's widget rects, so
        // a press must not arrive on the same frame the pointer first appears.
        let pos = frame(0.0, Vec::new(), &mut total).center();
        frame(0.0, vec![egui::Event::PointerMoved(pos)], &mut total);
        assert_eq!(total, 0, "hovering must not step");

        frame(
            0.0,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
            &mut total,
        );

        let mut t = 0.0;
        while t < seconds {
            t += 1.0 / 60.0;
            frame(t, Vec::new(), &mut total);
        }
        total
    }

    /// Regression, end to end through egui: the repeat used to be driven by
    /// `Response::is_pointer_button_down_on`, which egui clears once a press
    /// exceeds `max_click_duration` (0.8 s). That capped every hold at ~17
    /// steps and forced the user to keep re-pressing.
    #[test]
    fn a_held_button_in_egui_keeps_stepping_past_the_click_window() {
        // Under the old behaviour this died at egui's 0.8 s click window,
        // having produced about 17 steps in total.
        let steps = steps_while_held_in_egui(4.0);
        assert!(
            steps > 400,
            "four seconds of holding produced only {steps} steps"
        );
    }

    /// The counterpart: a press released straight away is still one step, so
    /// tracking the raw pointer button did not turn a click into a burst.
    #[test]
    fn a_quick_click_in_egui_is_a_single_step() {
        let steps = steps_while_held_in_egui(DELAY - 0.05);
        assert_eq!(steps, 1);
    }

    #[test]
    fn rate_is_zero_during_the_delay_then_ramps_between_the_bounds() {
        assert_eq!(rate(0.0), 0.0);
        assert_eq!(rate(DELAY - 0.001), 0.0);
        assert_eq!(rate(DELAY), MIN_RATE);
        assert_eq!(rate(DELAY + RAMP), MAX_RATE);
        // Clamped, never faster than the top rate.
        assert_eq!(rate(DELAY + RAMP * 10.0), MAX_RATE);
        let mid = rate(DELAY + RAMP / 2.0);
        assert!(mid > MIN_RATE && mid < MAX_RATE, "midpoint rate was {mid}");
    }
}
