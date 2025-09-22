use crate::reqwest_tower_service::ReqwestTowerService;
use octocrab::{AuthState, LayerReady, NoConfig};

pub fn wasm_builder()
-> octocrab::OctocrabBuilder<ReqwestTowerService, NoConfig, AuthState, LayerReady> {
    let reqwest_client = ReqwestTowerService {
        base_url: Some(("https".parse().unwrap(), "api.github.com".parse().unwrap())),
        client: reqwest::Client::new(),
    };

    let builder = octocrab::OctocrabBuilder::new_empty()
        .with_service(reqwest_client)
        .with_executor(Box::new(wasm_bindgen_futures::spawn_local))
        .with_auth(AuthState::None);

    builder
}
