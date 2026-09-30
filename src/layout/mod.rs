mod slots;

use std::{fmt, time::Duration};

use crate::scene::{Slot, TextNode, TextScene};
use slots::{HorizontalAlignment, VerticalPlacement, slot_layout};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutMetrics {
    pub safe_margin: f32,
    pub crosshair_radius: f32,
    pub inter_item_gap: f32,
}

impl Default for LayoutMetrics {
    fn default() -> Self {
        Self {
            safe_margin: 24.0,
            crosshair_radius: 96.0,
            inter_item_gap: 6.0,
        }
    }
}

impl LayoutMetrics {
    pub fn validate(self) -> Result<(), InvalidLayoutMetrics> {
        for (name, value) in [
            ("safe margin", self.safe_margin),
            ("crosshair radius", self.crosshair_radius),
            ("inter-item gap", self.inter_item_gap),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(InvalidLayoutMetrics { name, value });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalSize {
    pub width: f32,
    pub height: f32,
}

impl LogicalSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalPosition {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedTextSize {
    pub width: f32,
    pub height: f32,
}

impl ShapedTextSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

pub trait TextMeasurer {
    type Error;

    fn measure(&self, node: &TextNode) -> Result<ShapedTextSize, Self::Error>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionedTextNode {
    pub node: TextNode,
    pub position: LogicalPosition,
    pub size: ShapedTextSize,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionedTextScene {
    pub revision: u64,
    pub nodes: Vec<PositionedTextNode>,
    /// Time from `elapsed` until the next blink visibility transition.
    pub next_animation_in: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InvalidLayoutMetrics {
    pub name: &'static str,
    pub value: f32,
}

impl fmt::Display for InvalidLayoutMetrics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} must be finite and non-negative, got {}",
            self.name, self.value
        )
    }
}

impl std::error::Error for InvalidLayoutMetrics {}

#[derive(Debug)]
pub enum LayoutError<E> {
    InvalidMetrics(InvalidLayoutMetrics),
    InvalidOutputSize(LogicalSize),
    Measure { id: String, source: E },
    InvalidShapedSize { id: String, size: ShapedTextSize },
}

impl<E: fmt::Display> fmt::Display for LayoutError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetrics(error) => error.fmt(formatter),
            Self::InvalidOutputSize(size) => write!(
                formatter,
                "logical output size must be finite and non-negative, got {}x{}",
                size.width, size.height
            ),
            Self::Measure { id, source } => {
                write!(formatter, "failed to measure text node {id:?}: {source}")
            }
            Self::InvalidShapedSize { id, size } => write!(
                formatter,
                "text node {id:?} has invalid shaped size {}x{}",
                size.width, size.height
            ),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for LayoutError<E> {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutEngine {
    metrics: LayoutMetrics,
}

impl LayoutEngine {
    pub fn new(metrics: LayoutMetrics) -> Result<Self, InvalidLayoutMetrics> {
        metrics.validate()?;
        Ok(Self { metrics })
    }

    pub const fn metrics(&self) -> LayoutMetrics {
        self.metrics
    }

    pub fn layout<M: TextMeasurer>(
        &self,
        scene: &TextScene,
        output: LogicalSize,
        elapsed: Duration,
        measurer: &M,
    ) -> Result<PositionedTextScene, LayoutError<M::Error>> {
        self.metrics
            .validate()
            .map_err(LayoutError::InvalidMetrics)?;
        if !output.width.is_finite()
            || !output.height.is_finite()
            || output.width < 0.0
            || output.height < 0.0
        {
            return Err(LayoutError::InvalidOutputSize(output));
        }

        let mut measured = Vec::with_capacity(scene.nodes.len());
        let mut next_animation_in = None;
        for node in &scene.nodes {
            let size = measurer
                .measure(node)
                .map_err(|source| LayoutError::Measure {
                    id: node.id.clone(),
                    source,
                })?;
            if !size.width.is_finite()
                || !size.height.is_finite()
                || size.width < 0.0
                || size.height < 0.0
            {
                return Err(LayoutError::InvalidShapedSize {
                    id: node.id.clone(),
                    size,
                });
            }
            let (visible, transition) = blink_state(elapsed, node.style.blink_hz);
            next_animation_in = min_duration(next_animation_in, transition);
            measured.push((size, visible));
        }

        let mut positions = vec![LogicalPosition { x: 0.0, y: 0.0 }; scene.nodes.len()];
        for slot in Slot::ALL {
            let indices = scene
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| (node.slot == slot).then_some(index))
                .collect::<Vec<_>>();
            if indices.is_empty() {
                continue;
            }
            position_slot(
                slot,
                &indices,
                &measured,
                &mut positions,
                output,
                self.metrics,
            );
        }

        let nodes = scene
            .nodes
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, node)| PositionedTextNode {
                node,
                position: positions[index],
                size: measured[index].0,
                visible: measured[index].1,
            })
            .collect();

        Ok(PositionedTextScene {
            revision: scene.revision,
            nodes,
            next_animation_in,
        })
    }
}

fn position_slot(
    slot: Slot,
    indices: &[usize],
    measured: &[(ShapedTextSize, bool)],
    positions: &mut [LogicalPosition],
    output: LogicalSize,
    metrics: LayoutMetrics,
) {
    let definition = slot_layout(
        slot,
        output.width,
        output.height,
        metrics.safe_margin,
        metrics.crosshair_radius,
    );
    let total_height = indices
        .iter()
        .map(|&index| measured[index].0.height)
        .sum::<f32>()
        + metrics.inter_item_gap * indices.len().saturating_sub(1) as f32;
    let mut cursor_y = match definition.placement {
        VerticalPlacement::Down => definition.anchor_y,
        VerticalPlacement::Centered => definition.anchor_y - total_height / 2.0,
        VerticalPlacement::Up => definition.anchor_y,
    };

    for &index in indices {
        let size = measured[index].0;
        let x = match definition.alignment {
            HorizontalAlignment::Left => definition.anchor_x,
            HorizontalAlignment::Center => definition.anchor_x - size.width / 2.0,
            HorizontalAlignment::Right => definition.anchor_x - size.width,
        };
        let y = match definition.placement {
            VerticalPlacement::Up => {
                cursor_y -= size.height;
                cursor_y
            }
            VerticalPlacement::Down | VerticalPlacement::Centered => cursor_y,
        };
        positions[index] = LogicalPosition { x, y };
        match definition.placement {
            VerticalPlacement::Up => cursor_y -= metrics.inter_item_gap,
            VerticalPlacement::Down | VerticalPlacement::Centered => {
                cursor_y += size.height + metrics.inter_item_gap;
            }
        }
    }
}

fn blink_state(elapsed: Duration, blink_hz: Option<f32>) -> (bool, Option<Duration>) {
    let Some(blink_hz) = blink_hz else {
        return (true, None);
    };
    // Scene validation guarantees a finite, positive frequency. Be defensive
    // here because layout is a public renderer boundary and may receive a scene
    // assembled directly in Rust.
    if !blink_hz.is_finite() || blink_hz <= 0.0 {
        return (true, None);
    }

    let half_period = 0.5 / f64::from(blink_hz);
    let elapsed_seconds = elapsed.as_secs_f64();
    let phase = (elapsed_seconds / half_period).floor();
    let visible = phase.rem_euclid(2.0) == 0.0;
    let next_transition = (phase + 1.0) * half_period;
    let remaining = (next_transition - elapsed_seconds).max(f64::EPSILON);
    (visible, Some(Duration::from_secs_f64(remaining)))
}

fn min_duration(current: Option<Duration>, candidate: Option<Duration>) -> Option<Duration> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.min(candidate)),
        (Some(current), None) => Some(current),
        (None, candidate) => candidate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Slot, TextStyle};
    use std::{collections::HashMap, convert::Infallible};

    struct Sizes(HashMap<String, ShapedTextSize>);

    impl TextMeasurer for Sizes {
        type Error = Infallible;

        fn measure(&self, node: &TextNode) -> Result<ShapedTextSize, Self::Error> {
            Ok(self.0[&node.id])
        }
    }

    fn node(id: &str, slot: Slot) -> TextNode {
        TextNode {
            id: id.to_owned(),
            slot,
            text: id.to_owned(),
            style: TextStyle::default(),
        }
    }

    fn layout(nodes: Vec<TextNode>, sizes: &[(&str, f32, f32)]) -> PositionedTextScene {
        let sizes = Sizes(
            sizes
                .iter()
                .map(|(id, width, height)| ((*id).to_owned(), ShapedTextSize::new(*width, *height)))
                .collect(),
        );
        LayoutEngine::new(LayoutMetrics {
            safe_margin: 10.0,
            crosshair_radius: 100.0,
            inter_item_gap: 5.0,
        })
        .unwrap()
        .layout(
            &TextScene { revision: 7, nodes },
            LogicalSize::new(1000.0, 800.0),
            Duration::ZERO,
            &sizes,
        )
        .unwrap()
    }

    #[test]
    fn all_slots_have_deterministic_finite_coordinates() {
        let nodes = Slot::ALL
            .into_iter()
            .enumerate()
            .map(|(index, slot)| node(&index.to_string(), slot))
            .collect::<Vec<_>>();
        let sizes = (0..Slot::ALL.len())
            .map(|index| (index.to_string(), 20.0, 10.0))
            .collect::<Vec<_>>();
        let size_refs = sizes
            .iter()
            .map(|(id, width, height)| (id.as_str(), *width, *height))
            .collect::<Vec<_>>();

        let first = layout(nodes.clone(), &size_refs);
        let second = layout(nodes, &size_refs);
        assert_eq!(first, second);
        assert!(
            first
                .nodes
                .iter()
                .all(|node| { node.position.x.is_finite() && node.position.y.is_finite() })
        );
    }

    #[test]
    fn crosshair_uses_output_center_and_shaped_width() {
        let result = layout(
            vec![node("top", Slot::CrosshairTop)],
            &[("top", 80.0, 20.0)],
        );
        assert_eq!(
            result.nodes[0].position,
            LogicalPosition { x: 460.0, y: 280.0 }
        );
    }

    #[test]
    fn screen_edges_respect_safe_margin_and_declaration_order() {
        let result = layout(
            vec![
                node("first", Slot::ScreenTopRight),
                node("other", Slot::ScreenBottomLeft),
                node("second", Slot::ScreenTopRight),
            ],
            &[
                ("first", 100.0, 20.0),
                ("other", 40.0, 10.0),
                ("second", 60.0, 30.0),
            ],
        );
        assert_eq!(
            result.nodes[0].position,
            LogicalPosition { x: 890.0, y: 10.0 }
        );
        assert_eq!(
            result.nodes[2].position,
            LogicalPosition { x: 930.0, y: 35.0 }
        );
        assert_eq!(
            result.nodes[1].position,
            LogicalPosition { x: 10.0, y: 780.0 }
        );
        assert_eq!(
            result
                .nodes
                .iter()
                .map(|node| node.node.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "other", "second"]
        );
    }

    #[test]
    fn logical_layout_is_independent_of_buffer_scale() {
        let nodes = vec![node("ias", Slot::CrosshairRight)];
        let first = layout(nodes.clone(), &[("ias", 80.0, 20.0)]);
        let second = layout(nodes, &[("ias", 80.0, 20.0)]);
        assert_eq!(first.nodes[0].position, second.nodes[0].position);
    }

    #[test]
    fn blink_only_changes_visibility_and_reports_next_transition() {
        let mut blinking = node("warning", Slot::CrosshairTop);
        blinking.style.blink_hz = Some(2.0);
        let scene = TextScene {
            revision: 1,
            nodes: vec![blinking],
        };
        let sizes = Sizes(HashMap::from([(
            "warning".to_owned(),
            ShapedTextSize::new(100.0, 20.0),
        )]));
        let engine = LayoutEngine::default();

        let visible = engine
            .layout(
                &scene,
                LogicalSize::new(1000.0, 800.0),
                Duration::ZERO,
                &sizes,
            )
            .unwrap();
        let hidden = engine
            .layout(
                &scene,
                LogicalSize::new(1000.0, 800.0),
                Duration::from_millis(250),
                &sizes,
            )
            .unwrap();
        assert!(visible.nodes[0].visible);
        assert!(!hidden.nodes[0].visible);
        assert_eq!(visible.nodes[0].position, hidden.nodes[0].position);
        assert_eq!(visible.next_animation_in, Some(Duration::from_millis(250)));
    }
}
