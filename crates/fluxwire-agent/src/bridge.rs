//! The seam between the tool layer and the live app (ADR-009). `fluxwire-ui` implements it in
//! M9.3; the headless server has none, so the `ui` class is never exposed there.

/// Calls cross to the GPUI thread inside the implementation; the tool layer never touches
/// GPUI entities. Methods arrive with `ui.*` tools in M9.3.
pub trait UiBridge: Send + Sync {
    /// A short name of the host window, for `server_info`.
    fn describe(&self) -> String;
}
