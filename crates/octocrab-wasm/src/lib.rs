#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use wasm::wasm_builder;

#[cfg(not(target_arch = "wasm32"))]
pub fn builder() -> octocrab::OctocrabBuilder {
    octocrab::OctocrabBuilder::new()
}

#[cfg(target_arch = "wasm32")]
pub use wasm::wasm_builder as builder;
