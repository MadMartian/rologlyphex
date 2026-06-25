//! Overlay model: the layer / group / button structures the overlay renders. Built by the
//! `layers` module from `layers.toml` and read by `overlay`.

#[derive(Debug, Clone)]
pub struct ButtonLegend {
    /// The glyph shown for this button (in render order within its group).
    pub display: String,
}

#[derive(Debug, Clone)]
pub struct ButtonGroup {
    /// Group label shown above the buttons; `None` for the unlabeled group.
    pub label: Option<String>,
    pub buttons: Vec<ButtonLegend>,
}

#[derive(Debug, Clone)]
pub struct LayoutInfo {
    /// Human title for the layer (the deck name).
    pub label: String,
    pub groups: Vec<ButtonGroup>,
}
