use crate::{
    config::{Corner, Modifier, Settings},
    focus::Display,
};

#[derive(Clone, Copy, Default)]
pub struct Pointer {
    pub x: i32,
    pub y: i32,
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
    pub down: bool,
    #[cfg(target_os = "macos")]
    pub primary_down: bool,
}

impl Pointer {
    fn has(self, modifier: Modifier) -> bool {
        match modifier {
            Modifier::None => !self.shift && !self.control && !self.alt && !self.super_key,
            Modifier::Shift => self.shift,
            Modifier::Control => self.control,
            Modifier::Alt => self.alt,
            Modifier::Super => self.super_key,
            Modifier::Off => false,
        }
    }
}

#[derive(Default)]
pub struct Gestures {
    previous: Option<Pointer>,
    turns: Vec<u64>,
    direction: i32,
    travel: u32,
    cooldown: u64,
    peek_until: u64,
    peek_modifier: Option<Modifier>,
    corner: bool,
}

#[derive(Default)]
pub struct GestureState {
    pub toggle: bool,
    pub paused: bool,
}

impl Gestures {
    pub fn peek(&mut self, now: u64) {
        self.peek_until = now + 2_000;
        self.peek_modifier = None;
    }

    pub fn update(
        &mut self,
        now: u64,
        pointer: Pointer,
        displays: &[Display],
        settings: &Settings,
    ) -> GestureState {
        let previous = self.previous.replace(pointer).unwrap_or(pointer);
        let dx = pointer.x - previous.x;
        let dy = pointer.y - previous.y;

        let mut toggle = false;
        let minimum = 38 - i32::from(settings.sensitivity) / 4;
        if settings.shake && !pointer.down && now >= self.cooldown && dx.abs() >= dy.abs() {
            self.turns.retain(|turn| now - turn <= 420);
            if self.turns.is_empty() && self.travel > 240 {
                self.travel = 0;
            }
            if dx.abs() >= minimum {
                let direction = dx.signum();
                if direction != self.direction && self.direction != 0 {
                    self.turns.push(now);
                }
                self.direction = direction;
                self.travel += dx.unsigned_abs();
            }
            if self.turns.len() >= 3 && self.travel >= (minimum as u32) * 5 {
                self.turns.clear();
                self.travel = 0;
                self.direction = 0;
                self.cooldown = now + 650;
                if pointer.has(settings.shake_peek) {
                    self.peek(now);
                    self.peek_modifier = Some(settings.shake_peek);
                } else {
                    toggle = pointer.has(settings.shake_toggle);
                }
            }
        }

        let in_corner = settings.corner != Corner::Off
            && displays.iter().any(|display| {
                let bounds = display.bounds;
                if !bounds.contains(pointer.x, pointer.y) {
                    return false;
                }
                let left = pointer.x - bounds.x < 12;
                let right = bounds.right() - pointer.x <= 12;
                let top = pointer.y - bounds.y < 12;
                let bottom = bounds.bottom() - pointer.y <= 12;
                match settings.corner {
                    Corner::TopLeft => top && left,
                    Corner::TopRight => top && right,
                    Corner::BottomLeft => bottom && left,
                    Corner::BottomRight => bottom && right,
                    Corner::Off => false,
                }
            });

        if in_corner && !self.corner && !pointer.down && (dx != 0 || dy != 0) {
            self.peek(now);
        }
        self.corner = in_corner;

        if self
            .peek_modifier
            .is_some_and(|modifier| !pointer.has(modifier))
        {
            self.peek_modifier = None;
        }

        GestureState {
            toggle,
            paused: now < self.peek_until || self.peek_modifier.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_the_mouse_while_moving_keeps_focus_active() {
        let mut gestures = Gestures::default();
        let settings: Settings = serde_json::from_str(r#"{"pause_dragging":true}"#).unwrap();
        for (time, x) in [(0, 10), (16, 90), (32, 160)] {
            let state = gestures.update(
                time,
                Pointer {
                    x,
                    down: true,
                    ..Pointer::default()
                },
                &[],
                &settings,
            );
            assert!(!state.paused && !state.toggle);
        }
    }

    #[test]
    fn shake_peek_extends_until_shift_is_released() {
        let mut detector = Gestures::default();
        let settings = Settings::default();
        let mut outcome = GestureState::default();
        for (step, x) in [0, 100, 0, 100, 0].into_iter().enumerate() {
            outcome = detector.update(
                step as u64 * 50,
                Pointer {
                    x,
                    shift: true,
                    ..Pointer::default()
                },
                &[],
                &settings,
            );
        }
        assert!(outcome.paused && !outcome.toggle);
        assert!(
            detector
                .update(
                    3_000,
                    Pointer {
                        shift: true,
                        ..Pointer::default()
                    },
                    &[],
                    &settings
                )
                .paused
        );
        assert!(
            !detector
                .update(3_020, Pointer::default(), &[], &settings)
                .paused
        );
    }
}
