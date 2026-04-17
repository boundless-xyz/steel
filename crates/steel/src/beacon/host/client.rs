// Copyright 2026 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A simple Beacon API client.

use alloy::transports::http::reqwest;
use alloy_primitives::B256;
use context_deserialize::ContextDeserialize;
use lighthouse_types::{
    ExecPayload, ForkName, FullPayload, MainnetEthSpec, SignedBeaconBlock, SignedBeaconBlockHeader,
};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use std::{collections::HashMap, fmt::Display, result::Result as StdResult};
use url::Url;

/// Errors returned by the [BeaconClient].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("could not parse URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("block does not contain an execution payload")]
    NoExecutionPayload,
    #[error("response is empty")]
    EmptyResponse,
}

/// Alias for Results returned by client methods.
pub type Result<T> = StdResult<T, Error>;

/// Response returned by the `get_block_header` API.
#[derive(Debug, Serialize, Deserialize)]
pub struct BlockHeaderResponse {
    pub root: B256,
    pub canonical: bool,
    pub header: SignedBeaconBlockHeader,
}

/// Generic wrapper structure for API responses containing data and metadata.
#[derive(Debug, Serialize, Deserialize)]
struct Response<T> {
    data: T,
    #[serde(flatten)]
    meta: HashMap<String, serde_json::Value>,
}

/// Concrete `SignedBeaconBlock` type for the mainnet preset.
pub type MainnetSignedBeaconBlock = SignedBeaconBlock<MainnetEthSpec, FullPayload<MainnetEthSpec>>;

/// Fork-versioned API response wrapper that uses Lighthouse's [ContextDeserialize] to
/// automatically dispatch to the correct fork variant based on the `version` field.
#[derive(Debug)]
struct ForkVersionedResponse<T> {
    data: T,
}

impl<'de, T: ContextDeserialize<'de, ForkName>> Deserialize<'de> for ForkVersionedResponse<T> {
    fn deserialize<D>(deserializer: D) -> StdResult<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper<'a> {
            version: ForkName,
            #[serde(borrow)]
            data: &'a serde_json::value::RawValue,
        }

        let helper = Helper::deserialize(deserializer)?;
        let mut de = serde_json::Deserializer::from_str(helper.data.get());
        let data =
            T::context_deserialize(&mut de, helper.version).map_err(serde::de::Error::custom)?;

        Ok(ForkVersionedResponse { data })
    }
}

/// Simple beacon API client for the `mainnet` preset that can query headers and blocks.
#[derive(Debug, Clone)]
pub struct BeaconClient {
    http: reqwest::Client,
    endpoint: Url,
}

impl BeaconClient {
    /// Creates a new beacon endpoint API client.
    pub fn new<U: reqwest::IntoUrl>(endpoint: U) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.into_url()?,
        })
    }

    async fn get_json<R: DeserializeOwned, T: Serialize>(
        &self,
        path: &str,
        query: Option<&T>,
    ) -> Result<R> {
        let target = self.endpoint.join(path)?;
        let mut builder = self.http.get(target);
        if let Some(query) = query {
            builder = builder.query(query)
        };
        let resp = builder.send().await?;
        let value = resp.error_for_status()?.json().await?;
        Ok(value)
    }

    /// Retrieves block details for the given block ID.
    ///
    /// Block ID can be 'head', 'genesis', 'finalized', <slot>, or <root>.
    pub async fn get_block(&self, block_id: impl Display) -> Result<MainnetSignedBeaconBlock> {
        let path = format!("eth/v2/beacon/blocks/{block_id}");
        let result: ForkVersionedResponse<MainnetSignedBeaconBlock> =
            self.get_json(&path, None::<&()>).await?;
        Ok(result.data)
    }

    /// Retrieves block header for the block identified by the given parent root.
    pub async fn get_header_for_parent_root(
        &self,
        parent_root: B256,
    ) -> Result<BlockHeaderResponse> {
        let path = "eth/v1/beacon/headers";
        let params = [("parent_root", parent_root)];
        let mut result: Response<Vec<BlockHeaderResponse>> =
            self.get_json(path, Some(&params)).await?;
        result.data.pop().ok_or(Error::EmptyResponse)
    }

    /// Retrieves the execution block hash for the given block id.
    pub async fn get_execution_payload_block_hash(&self, block_id: impl Display) -> Result<B256> {
        let block = self.get_block(block_id).await?;
        let block_hash = block
            .message()
            .execution_payload()
            .map_err(|_| Error::NoExecutionPayload)?
            .block_hash();
        Ok(block_hash.into_root())
    }
}
