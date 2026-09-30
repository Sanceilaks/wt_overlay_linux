use std::time::{Duration, Instant};

/// Renderer-independent state machine for damage, frame callbacks and sparse
/// animation timers.
#[derive(Clone, Debug, Default)]
pub struct RedrawState {
    configured: bool,
    dirty: bool,
    frame_callback_pending: bool,
    animation_deadline: Option<Instant>,
    reconfigure_pending: bool,
}

impl RedrawState {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn is_configured(&self) -> bool {
        self.configured
    }

    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub const fn is_frame_callback_pending(&self) -> bool {
        self.frame_callback_pending
    }

    pub const fn animation_deadline(&self) -> Option<Instant> {
        self.animation_deadline
    }

    /// Records the first usable layer-surface configure.
    pub fn configure(&mut self) {
        if !self.configured {
            self.configured = true;
            self.dirty = true;
            self.reconfigure_pending = true;
        }
    }

    pub fn mark_scene_changed(&mut self) {
        self.dirty = true;
    }

    pub fn mark_surface_changed(&mut self) {
        self.dirty = true;
        self.reconfigure_pending = true;
    }

    pub fn take_reconfigure(&mut self) -> bool {
        std::mem::take(&mut self.reconfigure_pending)
    }

    /// Replaces the animation deadline. Static scenes pass `None`.
    pub fn set_animation_deadline(&mut self, deadline: Option<Instant>) {
        self.animation_deadline = deadline;
    }

    pub fn set_animation_after(&mut self, now: Instant, delay: Option<Duration>) {
        self.animation_deadline = delay.and_then(|delay| now.checked_add(delay));
    }

    pub fn should_render(&self, now: Instant) -> bool {
        self.configured
            && !self.frame_callback_pending
            && (self.dirty
                || self
                    .animation_deadline
                    .is_some_and(|deadline| deadline <= now))
    }

    /// Reserves the one allowed frame callback and consumes current damage.
    /// The caller must request `wl_surface.frame` before presenting.
    pub fn begin_render(&mut self, now: Instant) -> bool {
        if !self.should_render(now) {
            return false;
        }
        self.dirty = false;
        self.frame_callback_pending = true;
        if self
            .animation_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            self.animation_deadline = None;
        }
        true
    }

    /// Called for the matching compositor callback.
    pub fn frame_callback_received(&mut self) {
        self.frame_callback_pending = false;
    }

    /// Releases a reservation when rendering failed before committing a frame.
    /// Damage remains set so a recoverable surface error can be retried.
    pub fn render_aborted(&mut self) {
        self.frame_callback_pending = false;
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_and_many_scene_notifications_coalesce() {
        let now = Instant::now();
        let mut redraw = RedrawState::new();
        assert!(!redraw.should_render(now));

        redraw.configure();
        redraw.mark_scene_changed();
        redraw.mark_scene_changed();
        assert!(redraw.should_render(now));
        assert!(redraw.begin_render(now));
        assert!(!redraw.begin_render(now));
    }

    #[test]
    fn static_scene_stops_after_frame_callback() {
        let now = Instant::now();
        let mut redraw = RedrawState::new();
        redraw.configure();
        assert!(redraw.begin_render(now));
        redraw.frame_callback_received();
        assert!(!redraw.should_render(now));
    }

    #[test]
    fn scene_change_while_callback_is_pending_is_not_lost() {
        let now = Instant::now();
        let mut redraw = RedrawState::new();
        redraw.configure();
        assert!(redraw.begin_render(now));
        redraw.mark_scene_changed();
        assert!(!redraw.should_render(now));
        redraw.frame_callback_received();
        assert!(redraw.should_render(now));
    }

    #[test]
    fn blinking_wakes_only_at_transition_deadline() {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(125);
        let mut redraw = RedrawState::new();
        redraw.configure();
        assert!(redraw.begin_render(now));
        redraw.frame_callback_received();
        redraw.set_animation_deadline(Some(deadline));

        assert!(!redraw.should_render(now + Duration::from_millis(124)));
        assert!(redraw.begin_render(deadline));
        assert_eq!(redraw.animation_deadline(), None);
    }

    #[test]
    fn resize_requests_reconfiguration_and_redraw() {
        let now = Instant::now();
        let mut redraw = RedrawState::new();
        redraw.configure();
        assert!(redraw.take_reconfigure());
        assert!(!redraw.take_reconfigure());
        assert!(redraw.begin_render(now));
        redraw.frame_callback_received();

        redraw.mark_surface_changed();
        assert!(redraw.take_reconfigure());
        assert!(redraw.should_render(now));
    }
}
