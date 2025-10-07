use alloy::{hex, primitives::address};
use commonware_cryptography::{PrivateKeyExt, Signer};
use indexmap::IndexMap;
use rand::SeedableRng;
use reth_network_peers::TrustedPeer;
use tempo_commonware_node_config::Config;
use tempo_commonware_node_cryptography::PrivateKey;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, runners::AsyncRunner};
use uuid::Uuid;

pub struct Node {
    #[allow(unused)]
    name: String,
    #[allow(unused)]
    commonware_config: Config,
    #[allow(unused)]
    public_key: String,
    container: ContainerAsync<GenericImage>,
}

impl Node {
    pub async fn get_eth_rpc_url(&self) -> eyre::Result<String> {
        let port = self.container.get_host_port_ipv4(8545).await?;
        Ok(format!("http://localhost:{port}"))
    }
}

pub async fn setup_validators(count: usize) -> Vec<Node> {
    let prefix = &Uuid::new_v4().to_string()[..8];
    let hostnames = (1..count)
        .map(|i| format!("{}-node-{i}", prefix))
        .collect::<Vec<_>>();

    let configs = generate_commonware_config(hostnames.clone());

    let mut nodes = Vec::new();

    let reth_peers = hostnames
        .iter()
        .map(|hostname| {
            let secret_key = secp256k1::SECP256K1
                .generate_keypair(&mut rand::thread_rng())
                .0;

            (
                secret_key,
                TrustedPeer::from_secret_key(
                    url::Host::parse(hostname).unwrap(),
                    30303,
                    &secret_key,
                ),
            )
        })
        .collect::<Vec<_>>();

    let trusted_peers = reth_peers
        .iter()
        .map(|(_, peer)| format!("{}", peer))
        .collect::<Vec<_>>()
        .join(",");

    for (config, reth_peer) in configs.into_iter().zip(reth_peers) {
        let discovery_secret = hex::encode(reth_peer.0.secret_bytes());

        let container = GenericImage::new("tempo-node", "latest")
            .with_exposed_port(8000.into())
            .with_exposed_port(8545.into())
            .with_exposed_port(8546.into())
            .with_exposed_port(30303.into())
            .with_copy_to(
                "/genesis.json",
                std::fs::canonicalize("tests/assets/test-genesis.json").unwrap(),
            )
            .with_copy_to(
                "/cw.toml",
                toml::to_string_pretty(&config.2)
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
            )
            .with_copy_to("/p2p.key", discovery_secret.as_bytes().to_vec())
            .with_cmd(vec![
                "node",
                "--chain",
                "/genesis.json",
                "--consensus-config",
                "/cw.toml",
                "--http",
                "--http.addr",
                "0.0.0.0",
                "--http.api",
                "all",
                "--port",
                "30303",
                "--metrics",
                "9000",
                "--engine.legacy-state-root",
                "--engine.disable-precompile-cache",
                "--trusted-peers",
                &trusted_peers,
                "--p2p-secret-key",
                "/p2p.key",
            ])
            .with_env_var("RUST_LOG", "debug")
            .with_container_name(config.0.clone())
            .with_network(prefix)
            .start()
            .await
            .unwrap();

        nodes.push(Node {
            name: config.0,
            public_key: config.1,
            commonware_config: config.2,
            container,
        });
    }

    nodes
}

fn generate_commonware_config(hostnames: Vec<String>) -> Vec<(String, String, Config)> {
    let seed = Some(10);
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed.unwrap_or_else(rand::random::<u64>));
    let mut signers = (0..hostnames.len())
        .map(|_| PrivateKey::from_rng(&mut rng))
        .collect::<Vec<_>>();
    signers.sort_by_key(|signer| signer.public_key());

    let all_peers: Vec<_> = signers.iter().map(|signer| signer.public_key()).collect();

    let bootstrappers = all_peers.iter().take(1).cloned().collect::<Vec<_>>();

    // generate consensus key
    let threshold = commonware_utils::quorum(4);
    let (polynomial, shares) = commonware_cryptography::bls12381::dkg::ops::generate_shares::<
        _,
        tempo_commonware_node_cryptography::BlsScheme,
    >(&mut rng, None, all_peers.len() as u32, threshold);

    // Generate instance configurations
    let mut these_will_be_peers = IndexMap::new();
    let mut configurations = Vec::new();

    for ((signer, share), hostname) in signers
        .into_iter()
        .zip(shares)
        .collect::<Vec<_>>()
        .into_iter()
        .zip(hostnames)
    {
        // Create peer config
        let name = signer.public_key().to_string();
        these_will_be_peers.insert(signer.public_key(), format!("{hostname}:8000"));
        let peer_config = Config {
            signer,
            share,
            polynomial: polynomial.clone(),
            listen_port: 8000,
            metrics_port: Some(8001),
            p2p: Default::default(),
            storage_directory: camino::absolute_utf8("/commonware")
                .expect("this should always be a valid directory"),
            worker_threads: 3,
            // this will be updated after we have collected all peers
            peers: IndexMap::new(),
            bootstrappers: bootstrappers.clone().into(),
            message_backlog: 16384,
            mailbox_size: 16384,
            deque_size: 10,
            fee_recipient: address!("0x0000000000000000000000000000000000000000"),
            timeouts: Default::default(),
        };
        configurations.push((hostname, name, peer_config));
    }

    configurations
        .iter_mut()
        .for_each(|(_, _, cfg)| cfg.peers = these_will_be_peers.clone());

    configurations
}
