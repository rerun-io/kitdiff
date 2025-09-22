#[cfg(target_arch = "wasm32")]
pub mod wasm;
#[cfg(target_arch = "wasm32")]
mod reqwest_tower_service;

#[cfg(target_arch = "wasm32")]
pub use wasm::wasm_builder;

#[cfg(not(target_arch = "wasm32"))]
use octocrab::{DefaultOctocrabBuilderConfig, NoAuth, NoSvc, NotLayerReady};
#[cfg(not(target_arch = "wasm32"))]
pub fn builder()
-> octocrab::OctocrabBuilder<NoSvc, DefaultOctocrabBuilderConfig, NoAuth, NotLayerReady> {
    octocrab::Octocrab::builder()
}

#[cfg(target_arch = "wasm32")]
pub use wasm::wasm_builder as builder;
