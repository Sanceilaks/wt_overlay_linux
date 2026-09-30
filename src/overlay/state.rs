use smithay_client_toolkit::{
    compositor::{FrameCallbackData, Region},
    output::OutputState,
    registry::RegistryState,
    shell::{WaylandSurface, wlr_layer::LayerSurface},
};
use wayland_client::{
    QueueHandle,
    protocol::{wl_output, wl_surface::WlSurface},
};

pub struct OverlayState {
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    layer_surface: Option<LayerSurface>,
    input_region: Option<Region>,
    pub selected_output: Option<wl_output::WlOutput>,
    pub selected_output_name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub configured: bool,
    pub exit: bool,
    pub resized: bool,
    pub dirty: bool,
    pub frame_callback_pending: bool,
    pub queue_handle: Option<QueueHandle<Self>>,
}

impl OverlayState {
    pub fn bootstrap(registry_state: RegistryState, output_state: OutputState) -> Self {
        Self {
            registry_state,
            output_state,
            layer_surface: None,
            input_region: None,
            selected_output: None,
            selected_output_name: None,
            width: 1,
            height: 1,
            scale: 1,
            configured: false,
            exit: false,
            resized: false,
            dirty: false,
            frame_callback_pending: false,
            queue_handle: None,
        }
    }

    pub fn named_outputs(&self) -> Vec<(wl_output::WlOutput, String)> {
        self.output_state
            .outputs()
            .filter_map(|output| {
                self.output_state
                    .info(&output)
                    .and_then(|info| info.name.map(|name| (output, name)))
            })
            .collect()
    }

    pub fn attach(
        &mut self,
        layer_surface: LayerSurface,
        input_region: Region,
        output: wl_output::WlOutput,
        output_name: String,
        scale: i32,
    ) {
        self.layer_surface = Some(layer_surface);
        self.input_region = Some(input_region);
        self.selected_output = Some(output);
        self.selected_output_name = Some(output_name);
        self.scale = scale;
    }

    pub fn apply_clickthrough(&self) {
        if let (Some(layer), Some(region)) = (&self.layer_surface, &self.input_region) {
            layer.set_input_region(Some(region.wl_region()));
        }
    }

    pub fn surface(&self) -> &WlSurface {
        self.layer_surface
            .as_ref()
            .expect("layer surface initialized")
            .wl_surface()
    }

    pub fn request_next_frame(&self, qh: &QueueHandle<Self>) {
        let surface = self.surface();
        surface.frame(qh, FrameCallbackData(surface.clone()));
    }
}
