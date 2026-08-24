use async_trait::async_trait;

use crate::domain::{AssetSpec, ChainAddress, TraceWindow, Transfer};

#[derive(Clone, Debug)]
pub struct TransferBatch {
    pub transfers: Vec<Transfer>,
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("upstream request failed: {0}")]
    Request(String),
    #[error("upstream rate limit reached")]
    RateLimited,
    #[error("upstream denied the request")]
    Forbidden,
    #[error("upstream response was invalid: {0}")]
    InvalidResponse(String),
    #[error("the selected chain is not configured")]
    UnsupportedChain,
    #[error("the selected chain provider is not configured on this server")]
    UnconfiguredChain,
    #[error("the selected asset is not supported by this provider")]
    UnsupportedAsset,
    #[error("the requested token metadata does not match the provider response")]
    AssetMetadataMismatch,
    #[error(
        "the requested trace window exceeds {provider}'s {retention_days}-day history retention"
    )]
    WindowOutsideRetention {
        provider: &'static str,
        retention_days: u16,
    },
}

#[async_trait]
pub trait TransferProvider: Send + Sync {
    async fn transfers(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        max_pages: usize,
    ) -> Result<TransferBatch, ProviderError>;
}
