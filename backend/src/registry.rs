use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use crate::{
    domain::{AssetSpec, ChainAddress, ChainFamily, TraceWindow},
    provider::{ProviderError, TransferBatch, TransferProvider},
};

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: HashMap<ChainFamily, Arc<dyn TransferProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, family: ChainFamily, provider: Arc<dyn TransferProvider>) {
        self.providers.insert(family, provider);
    }

    pub fn configured_families(&self) -> Vec<ChainFamily> {
        let mut families = self.providers.keys().copied().collect::<Vec<_>>();
        families.sort_by_key(|family| match family {
            ChainFamily::Utxo => 0,
            ChainFamily::Evm => 1,
            ChainFamily::Solana => 2,
            ChainFamily::Tron => 3,
            ChainFamily::Xrpl => 4,
        });
        families
    }
}

#[async_trait]
impl TransferProvider for ProviderRegistry {
    async fn transfers(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        max_pages: usize,
    ) -> Result<TransferBatch, ProviderError> {
        let provider = self
            .providers
            .get(&address.chain().family())
            .ok_or(ProviderError::UnconfiguredChain)?;
        provider.transfers(address, asset, window, max_pages).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_reports_configured_families_deterministically() {
        #[derive(Clone)]
        struct Noop;

        #[async_trait]
        impl TransferProvider for Noop {
            async fn transfers(
                &self,
                _address: &ChainAddress,
                _asset: &AssetSpec,
                _window: TraceWindow,
                _max_pages: usize,
            ) -> Result<TransferBatch, ProviderError> {
                Ok(TransferBatch {
                    transfers: Vec::new(),
                    truncated: false,
                })
            }
        }

        let mut registry = ProviderRegistry::new();
        registry.register(ChainFamily::Xrpl, Arc::new(Noop));
        registry.register(ChainFamily::Utxo, Arc::new(Noop));
        assert_eq!(
            registry.configured_families(),
            vec![ChainFamily::Utxo, ChainFamily::Xrpl]
        );
    }
}
