use crate::scene::Slot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HorizontalAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum VerticalPlacement {
    Down,
    Centered,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SlotLayout {
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub alignment: HorizontalAlignment,
    pub placement: VerticalPlacement,
}

pub(super) fn slot_layout(
    slot: Slot,
    width: f32,
    height: f32,
    safe_margin: f32,
    crosshair_radius: f32,
) -> SlotLayout {
    let left = safe_margin;
    let center_x = width / 2.0;
    let right = width - safe_margin;
    let top = safe_margin;
    let center_y = height / 2.0;
    let bottom = height - safe_margin;

    match slot {
        Slot::CrosshairTop => SlotLayout {
            anchor_x: center_x,
            anchor_y: center_y - crosshair_radius,
            alignment: HorizontalAlignment::Center,
            placement: VerticalPlacement::Up,
        },
        Slot::CrosshairRight => SlotLayout {
            anchor_x: center_x + crosshair_radius,
            anchor_y: center_y,
            alignment: HorizontalAlignment::Left,
            placement: VerticalPlacement::Centered,
        },
        Slot::CrosshairBottom => SlotLayout {
            anchor_x: center_x,
            anchor_y: center_y + crosshair_radius,
            alignment: HorizontalAlignment::Center,
            placement: VerticalPlacement::Down,
        },
        Slot::CrosshairLeft => SlotLayout {
            anchor_x: center_x - crosshair_radius,
            anchor_y: center_y,
            alignment: HorizontalAlignment::Right,
            placement: VerticalPlacement::Centered,
        },
        Slot::ScreenTopLeft | Slot::ScreenLeftTop => SlotLayout {
            anchor_x: left,
            anchor_y: top,
            alignment: HorizontalAlignment::Left,
            placement: VerticalPlacement::Down,
        },
        Slot::ScreenTopCenter => SlotLayout {
            anchor_x: center_x,
            anchor_y: top,
            alignment: HorizontalAlignment::Center,
            placement: VerticalPlacement::Down,
        },
        Slot::ScreenTopRight | Slot::ScreenRightTop => SlotLayout {
            anchor_x: right,
            anchor_y: top,
            alignment: HorizontalAlignment::Right,
            placement: VerticalPlacement::Down,
        },
        Slot::ScreenLeftMiddle => SlotLayout {
            anchor_x: left,
            anchor_y: center_y,
            alignment: HorizontalAlignment::Left,
            placement: VerticalPlacement::Centered,
        },
        Slot::ScreenRightMiddle => SlotLayout {
            anchor_x: right,
            anchor_y: center_y,
            alignment: HorizontalAlignment::Right,
            placement: VerticalPlacement::Centered,
        },
        Slot::ScreenBottomLeft | Slot::ScreenLeftBottom => SlotLayout {
            anchor_x: left,
            anchor_y: bottom,
            alignment: HorizontalAlignment::Left,
            placement: VerticalPlacement::Up,
        },
        Slot::ScreenBottomCenter => SlotLayout {
            anchor_x: center_x,
            anchor_y: bottom,
            alignment: HorizontalAlignment::Center,
            placement: VerticalPlacement::Up,
        },
        Slot::ScreenBottomRight | Slot::ScreenRightBottom => SlotLayout {
            anchor_x: right,
            anchor_y: bottom,
            alignment: HorizontalAlignment::Right,
            placement: VerticalPlacement::Up,
        },
    }
}
