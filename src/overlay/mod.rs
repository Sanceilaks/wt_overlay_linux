mod handlers;
mod state;

use std::{error::Error, time::Duration};

use smithay_client_toolkit::{
    compositor::{CompositorState, Region},
    output::OutputState,
    reexports::{
        calloop::{EventLoop, ping::PingSource},
        calloop_wayland_source::WaylandSource,
    },
    registry::RegistryState,
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell},
    },
};
use state::OverlayState;
use wayland_client::{Connection, EventQueue, globals::registry_queue_init};

pub trait OverlayRenderer {
    fn resize(&mut self, width: u32, height: u32, scale: i32);
    fn render(&mut self) -> Result<(), Box<dyn Error>>;
    fn next_animation_in(&self) -> Option<Duration> {
        None
    }
}

pub struct Overlay {
    conn: Connection,
    event_queue: EventQueue<OverlayState>,
    state: OverlayState,
}

impl Overlay {
    pub fn new(output_name: &str) -> Result<Self, Box<dyn Error>> {
        let conn = Connection::connect_to_env()
            .map_err(|error| format!("cannot connect to Wayland compositor: {error}"))?;
        let (globals, mut event_queue) = registry_queue_init(&conn)?;
        let qh = event_queue.handle();

        let compositor_state = CompositorState::bind(&globals, &qh)
            .map_err(|error| format!("wl_compositor is unavailable: {error}"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|error| format!("zwlr_layer_shell_v1 is unavailable: {error}"))?;
        let mut state = OverlayState::bootstrap(
            RegistryState::new(&globals),
            OutputState::new(&globals, &qh),
        );

        // `wl_output` and xdg-output information arrives asynchronously.
        event_queue.roundtrip(&mut state)?;
        event_queue.roundtrip(&mut state)?;
        let named_outputs = state.named_outputs();
        let output = named_outputs
            .iter()
            .find(|(_, name)| name == output_name)
            .map(|(output, _)| output.clone())
            .ok_or_else(|| {
                let available = named_outputs
                    .iter()
                    .map(|(_, name)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Wayland output '{output_name}' was not found (available: {}). Run wayland-info to list output names.",
                    if available.is_empty() { "<none>" } else { &available }
                )
            })?;

        let scale = state
            .output_state
            .info(&output)
            .map_or(1, |info| info.scale_factor.max(1));
        let surface = compositor_state.create_surface(&qh);
        surface.set_buffer_scale(scale);
        let layer_surface = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("wt_crosshair_hud"),
            Some(&output),
        );
        layer_surface.set_size(0, 0);
        layer_surface.set_anchor(Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        let input_region = Region::new(&compositor_state)?;
        layer_surface.set_input_region(Some(input_region.wl_region()));
        layer_surface.commit();
        state.attach(
            layer_surface,
            input_region,
            output,
            output_name.to_owned(),
            scale,
        );

        Ok(Self {
            conn,
            event_queue,
            state,
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn surface(&self) -> &wayland_client::protocol::wl_surface::WlSurface {
        self.state.surface()
    }

    pub fn size_and_scale(&self) -> (u32, u32, i32) {
        (self.state.width, self.state.height, self.state.scale)
    }

    /// Runs the Wayland source through calloop and renders only after configure,
    /// scene invalidation, resize, or a compositor frame callback.
    pub fn run(
        mut self,
        mut renderer: impl OverlayRenderer,
        scene_wake: PingSource,
    ) -> Result<(), Box<dyn Error>> {
        let mut event_loop: EventLoop<OverlayState> = EventLoop::try_new()?;
        WaylandSource::new(self.conn.clone(), self.event_queue).insert(event_loop.handle())?;
        event_loop
            .handle()
            .insert_source(scene_wake, |(), &mut (), state| {
                state.dirty = true;
            })?;

        while !self.state.exit {
            let timeout = renderer.next_animation_in();
            let waiting_since = std::time::Instant::now();
            event_loop.dispatch(timeout, &mut self.state)?;
            if timeout.is_some_and(|deadline| waiting_since.elapsed() >= deadline) {
                self.state.dirty = true;
            }

            if self.state.resized && self.state.configured {
                renderer.resize(self.state.width, self.state.height, self.state.scale);
                self.state.resized = false;
                self.state.dirty = true;
            }

            if self.state.configured && self.state.dirty && !self.state.frame_callback_pending {
                self.state
                    .request_next_frame(&self.state.queue_handle.clone().expect("queue handle"));
                self.state.frame_callback_pending = true;
                self.state.dirty = false;
                renderer.render()?;
            }
        }
        Ok(())
    }
}
