use std::{env, net::SocketAddr, sync::Arc};

use chaintrace_api::{
    etherscan::EtherscanClient, helius::HeliusClient, router, tatum::TatumUtxoClient,
    trongrid::TronGridClient, xrpl::XrplClient, ChainFamily, ProviderRegistry,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("chaintrace_api=info,tower_http=info")),
        )
        .init();

    let mut providers = ProviderRegistry::new();

    let base_url =
        env::var("TRONGRID_BASE_URL").unwrap_or_else(|_| "https://api.trongrid.io".to_owned());
    let api_key = optional_env("TRONGRID_API_KEY");
    let network = env::var("TRON_NETWORK").unwrap_or_else(|_| "tron-mainnet".to_owned());
    let trongrid = TronGridClient::new(base_url, api_key, network, 10)
        .expect("failed to initialize TronGrid client");
    providers.register(ChainFamily::Tron, Arc::new(trongrid));

    if let Some(api_key) = optional_env("ETHERSCAN_API_KEY") {
        let base_url = env::var("ETHERSCAN_BASE_URL")
            .unwrap_or_else(|_| "https://api.etherscan.io/v2/api".to_owned());
        providers.register(
            ChainFamily::Evm,
            Arc::new(
                EtherscanClient::new(base_url, api_key, 10)
                    .expect("failed to initialize Etherscan client"),
            ),
        );
    }
    if let Some(api_key) = optional_env("TATUM_API_KEY") {
        let base_url =
            env::var("TATUM_BASE_URL").unwrap_or_else(|_| "https://api.tatum.io".to_owned());
        providers.register(
            ChainFamily::Utxo,
            Arc::new(
                TatumUtxoClient::new(base_url, api_key, 10)
                    .expect("failed to initialize Tatum client"),
            ),
        );
    }
    if let Some(api_key) = optional_env("HELIUS_API_KEY") {
        let rpc_url = env::var("HELIUS_RPC_URL")
            .unwrap_or_else(|_| "https://mainnet.helius-rpc.com/".to_owned());
        providers.register(
            ChainFamily::Solana,
            Arc::new(
                HeliusClient::new(rpc_url, api_key, 10)
                    .expect("failed to initialize Helius client"),
            ),
        );
    }
    let xrpl_url =
        env::var("XRPL_RPC_URL").unwrap_or_else(|_| "https://s2.ripple.com:51234/".to_owned());
    providers.register(
        ChainFamily::Xrpl,
        Arc::new(XrplClient::new(xrpl_url, 10).expect("failed to initialize XRPL client")),
    );

    tracing::info!(families = ?providers.configured_families(), "configured chain provider families");
    let api_token = env::var("CHAINTRACE_API_TOKEN")
        .ok()
        .filter(|value| value.trim().len() >= 32)
        .expect("CHAINTRACE_API_TOKEN must be set to a secret of at least 32 characters");
    let app = router(Arc::new(providers), api_token);
    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind API listener");
    tracing::info!(%address, "ChainTrace API listening");
    axum::serve(listener, app).await.expect("API server failed");
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
