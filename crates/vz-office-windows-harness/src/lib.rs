//! Default-off, measurement-only Windows C1 harness. Never used by native launch.
#[cfg(all(windows, feature = "office-feasibility"))]
pub mod launch;
#[cfg(all(windows, feature = "office-feasibility"))]
pub mod probes;
