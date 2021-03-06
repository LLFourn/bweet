use bdk::{
    bitcoin::Network,
    blockchain::{esplora::EsploraBlockchainConfig, AnyBlockchainConfig},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde()]
    pub network: Network,
    pub blockchain: AnyBlockchainConfig,
}

impl Config {
    pub fn default_config(network: Network) -> Config {
        use Network::*;
        let concurrency = Some(4);
        let blockchain = match network {
            Bitcoin => AnyBlockchainConfig::Esplora(EsploraBlockchainConfig {
                base_url: "https://blockstream.info/api/".to_string(),
                concurrency,
            }),
            Testnet => AnyBlockchainConfig::Esplora(EsploraBlockchainConfig {
                base_url: "https://blockstream.info/testnet/api/".to_string(),
                concurrency,
            }),
            Regtest => AnyBlockchainConfig::Esplora(EsploraBlockchainConfig {
                base_url: "http://localhost:3000".to_string(),
                concurrency,
            }),
            Signet => unimplemented!("signet not supported yet!"),
        };

        Config {
            network,
            blockchain,
        }
    }
}
