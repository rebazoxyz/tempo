//! Definitions to read and write a tempo consensus configuration.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use std::{net::SocketAddr, path::Path};

use commonware_cryptography::{bls12381::primitives::group::Share, ed25519::PrivateKey};

pub mod p2p;
pub mod timeouts;

#[cfg(test)]
mod tests;

/// Configuration for the commonware consensus engine.
///
// TODO: There are plenty of other settings that could be added here. alto's `engine::Config`
// lists a number of hardcoded values, while also hardcoding a lot of other settings.
//
// + partition_prefix
// + blocks_freezer_table_initial_size
// + finalized_freezer_table_initial_size
// + backfill_quota
// + leader_timeout
// + notarization_timeout
// + nullify_retry
// + activity_timeout
// + skip_timeout
// + fetch_timeout
// + max_fetch_count
// + fetch_concurrent
// + fetch_rate_per_peer
// + pending_limit
// + recovered_limit
// + resolver_limit
// + broadcaster_limit
// + backfill_quota
// + namespace
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Config {
    #[serde(with = "crate::_serde::private_key")]
    pub signer: PrivateKey,

    #[serde(
        default,
        with = "crate::_serde::optional_share",
        skip_serializing_if = "Option::is_none"
    )]
    pub share: Option<Share>,

    /// Address on which the node listens. Supply `0.0.0.0:<port>` to listen
    /// on all addresses.
    pub listen_addr: SocketAddr,

    pub metrics_port: Option<u16>,

    pub p2p: p2p::Config,

    pub storage_directory: camino::Utf8PathBuf,
    pub worker_threads: usize,

    pub message_backlog: usize,
    pub mailbox_size: usize,
    pub deque_size: usize,

    pub fee_recipient: alloy_primitives::Address,

    /// Various timeouts employed by the consensus engine, both continuous
    /// and discrete time.
    #[serde(default)]
    pub timeouts: timeouts::Config,
}

impl Config {
    /// Parses [`Config`] from a toml formatted file at `path`.
    // TODO: also support json down the line because eth/reth chainspecs
    // are json? Maybe even replace toml? Toml is nicer for humans.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let file_contents = std::fs::read_to_string(path)?;
        let this = toml::from_str(&file_contents)?;
        Ok(this)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to open file for reading")]
    OpenFile(#[from] std::io::Error),
    #[error("failed parsing file contents")]
    Parse(#[from] toml::de::Error),
}

mod _serde {
    pub(crate) mod optional_share {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub(crate) fn serialize<S>(
            share: &Option<crate::Share>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            #[derive(Serialize)]
            struct Intermediate<'a>(#[serde(with = "crate::_serde::share")] &'a crate::Share);

            share.as_ref().map(Intermediate).serialize(serializer)
        }

        pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<crate::Share>, D::Error>
        where
            D: Deserializer<'de>,
        {
            #[derive(Deserialize)]
            struct Intermediate(#[serde(with = "crate::_serde::share")] crate::Share);

            let intermediate = Option::deserialize(deserializer)?;
            Ok(intermediate.map(|Intermediate(share)| share))
        }
    }

    pub(crate) mod share {
        use commonware_codec::{DecodeExt as _, Encode as _};
        use serde::{Deserializer, Serializer};

        pub(crate) fn serialize<S>(share: &crate::Share, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let bytes = share.encode();
            const_hex::serde::serialize(&bytes, serializer)
        }

        pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<crate::Share, D::Error>
        where
            D: Deserializer<'de>,
        {
            // XXX: we don't use commonware's built-in hex tooling because it doesn't provide good
            // errors. If it fails, `None` is all you get.
            let bytes: Vec<u8> = const_hex::serde::deserialize(deserializer)?;
            let share = crate::Share::decode(&bytes[..]).map_err(|err| {
                serde::de::Error::custom(format!(
                    "failed decoding hex-formatted bytes as group share: {err:?}"
                ))
            })?;
            Ok(share)
        }
    }

    pub(crate) mod private_key {
        use commonware_codec::{DecodeExt as _, Encode as _};
        use serde::{Deserializer, Serializer};

        pub(crate) fn serialize<S>(
            private_key: &crate::PrivateKey,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let bytes = private_key.encode();
            const_hex::serde::serialize(&bytes, serializer)
        }

        pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<crate::PrivateKey, D::Error>
        where
            D: Deserializer<'de>,
        {
            // XXX: we don't use commonware's built-in hex tooling because it doesn't provide good
            // errors. If it fails, `None` is all you get.
            let bytes: Vec<u8> = const_hex::serde::deserialize(deserializer)?;
            let signer = crate::PrivateKey::decode(&bytes[..]).map_err(|err| {
                serde::de::Error::custom(format!(
                    "failed decoding hex-formatted bytes as private key: {err:?}"
                ))
            })?;
            Ok(signer)
        }
    }
}
