use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use crate::{
    domain::{AssetSpec, Chain, ChainAddress, ChainFamily, TraceWindow},
    provider::{ProviderError, TransferBatch, TransferProvider},
};

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: HashMap<ChainFamily, Arc<dyn TransferProvider>>,
    chain_providers: HashMap<Chain, Arc<dyn TransferProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, family: ChainFamily, provider: Arc<dyn TransferProvider>) {
        self.providers.insert(family, provider);
    }

    pub fn register_chain(&mut self, chain: Chain, provider: Arc<dyn TransferProvider>) {
        self.chain_providers.insert(chain, provider);
    }

    pub fn configured_families(&self) -> Vec<ChainFamily> {
        let mut families = self.providers.keys().copied().collect::<Vec<_>>();
        for family in self.chain_providers.keys().map(|chain| chain.family()) {
            if !families.contains(&family) {
                families.push(family);
            }
        }
        families.sort_by_key(|family| match family {
            ChainFamily::Utxo => 0,
            ChainFamily::Evm => 1,
            ChainFamily::Solana => 2,
            ChainFamily::Tron => 3,
            ChainFamily::Xrpl => 4,
        });
        families
    }

    pub fn configured_chains(&self) -> Vec<Chain> {
        let mut chains = self.chain_providers.keys().copied().collect::<Vec<_>>();
        chains.sort();
        chains
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
            .chain_providers
            .get(&address.chain())
            .or_else(|| self.providers.get(&address.chain().family()))
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

    #[tokio::test]
    async fn chain_provider_overrides_its_family_provider_without_affecting_peers() {
        #[derive(Clone)]
        struct Empty;

        #[async_trait]
        impl TransferProvider for Empty {
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

        #[derive(Clone)]
        struct RateLimited;

        #[async_trait]
        impl TransferProvider for RateLimited {
            async fn transfers(
                &self,
                _address: &ChainAddress,
                _asset: &AssetSpec,
                _window: TraceWindow,
                _max_pages: usize,
            ) -> Result<TransferBatch, ProviderError> {
                Err(ProviderError::RateLimited)
            }
        }

        let mut registry = ProviderRegistry::new();
        registry.register(ChainFamily::Evm, Arc::new(Empty));
        registry.register_chain(Chain::EthereumClassic, Arc::new(RateLimited));
        assert_eq!(registry.configured_chains(), vec![Chain::EthereumClassic]);
        assert_eq!(registry.configured_families(), vec![ChainFamily::Evm]);

        let etc = ChainAddress::parse(
            Chain::EthereumClassic,
            "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
        )
        .unwrap();
        assert!(matches!(
            registry
                .transfers(
                    &etc,
                    &AssetSpec::native(),
                    TraceWindow {
                        min_timestamp: 0,
                        max_timestamp: 1,
                    },
                    1,
                )
                .await,
            Err(ProviderError::RateLimited)
        ));

        let ethereum = ChainAddress::parse(
            Chain::Ethereum,
            "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
        )
        .unwrap();
        assert!(registry
            .transfers(
                &ethereum,
                &AssetSpec::native(),
                TraceWindow {
                    min_timestamp: 0,
                    max_timestamp: 1,
                },
                1,
            )
            .await
            .unwrap()
            .transfers
            .is_empty());
    }
}
