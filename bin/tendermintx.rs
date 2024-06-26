//! To build the binary:
//!
//!     `cargo build --release --bin tendermintx`
//!

use std::env;

use alloy_primitives::{Address, Bytes, B256};
use alloy_sol_types::{sol, SolType};
use anyhow::Result;
use ethers::abi::AbiEncode;
use ethers::contract::abigen;
use ethers::providers::{Http, Provider};
use log::{error, info};
use subtle_encoding::hex;
use succinct_client::request::SuccinctClient;
use tendermintx::input::InputDataFetcher;

// Note: Update ABI when updating contract.
abigen!(TendermintX, "./abi/TendermintX.abi.json");

struct TendermintXConfig {
    address: Address,
    chain_id: u32,
    step_function_id: B256,
    skip_function_id: B256,
}

struct TendermintXOperator {
    config: TendermintXConfig,
    contract: TendermintX<Provider<Http>>,
    client: SuccinctClient,
    data_fetcher: InputDataFetcher,
}

type StepInputTuple = sol! { tuple(uint64, bytes32) };

type SkipInputTuple = sol! { tuple(uint64, bytes32, uint64) };

impl TendermintXOperator {
    pub fn new() -> Self {
        let config = Self::get_config();

        let ethereum_rpc_url = env::var("ETHEREUM_RPC_URL").expect("ETHEREUM_RPC_URL must be set");
        let provider =
            Provider::<Http>::try_from(ethereum_rpc_url).expect("could not connect to client");

        let contract = TendermintX::new(config.address.0 .0, provider.into());

        let data_fetcher = InputDataFetcher::default();

        let succinct_rpc_url = env::var("SUCCINCT_RPC_URL").expect("SUCCINCT_RPC_URL must be set");
        let succinct_api_key = env::var("SUCCINCT_API_KEY").expect("SUCCINCT_API_KEY must be set");
        let client = SuccinctClient::new(succinct_rpc_url, succinct_api_key, false, false);

        Self {
            config,
            contract,
            client,
            data_fetcher,
        }
    }

    fn get_config() -> TendermintXConfig {
        let contract_address = env::var("CONTRACT_ADDRESS").expect("CONTRACT_ADDRESS must be set");
        let chain_id = env::var("CHAIN_ID").expect("CHAIN_ID must be set");
        let address = contract_address
            .parse::<Address>()
            .expect("invalid address");

        // Load the function IDs.
        let step_id_env = env::var("STEP_FUNCTION_ID").expect("STEP_FUNCTION_ID must be set");
        let step_function_id = B256::from_slice(
            &hex::decode(step_id_env.strip_prefix("0x").unwrap_or(&step_id_env))
                .expect("invalid hex for step_function_id, expected 0x prefix"),
        );
        let skip_id_env = env::var("SKIP_FUNCTION_ID").expect("SKIP_FUNCTION_ID must be set");
        let skip_function_id = B256::from_slice(
            &hex::decode(skip_id_env.strip_prefix("0x").unwrap_or(&skip_id_env))
                .expect("invalid hex for skip_function_id, expected 0x prefix"),
        );

        TendermintXConfig {
            address,
            chain_id: chain_id.parse::<u32>().expect("invalid chain id"),
            step_function_id,
            skip_function_id,
        }
    }

    async fn request_step(&self, trusted_block: u64) -> Result<String> {
        let trusted_header_hash = self
            .contract
            .block_height_to_header_hash(trusted_block)
            .await
            .unwrap();

        let input = StepInputTuple::abi_encode_packed(&(trusted_block, trusted_header_hash));

        let step_call = StepCall { trusted_block };
        let function_data = step_call.encode();

        let request_id = self
            .client
            .submit_platform_request(
                self.config.chain_id,
                self.config.address,
                function_data.into(),
                self.config.step_function_id,
                Bytes::copy_from_slice(&input),
            )
            .await?;
        Ok(request_id)
    }

    async fn request_step_2(&self, trusted_hash: [u8; 32], trusted_block: u64) -> Result<String> {
        let trusted_header_hash = trusted_hash;

        let input = StepInputTuple::abi_encode_packed(&(trusted_block, trusted_header_hash));

        let step_call = StepCall { trusted_block };
        let function_data = step_call.encode();

        let request_id = self
            .client
            .submit_platform_request(
                self.config.chain_id,
                self.config.address,
                function_data.into(),
                self.config.step_function_id,
                Bytes::copy_from_slice(&input),
            )
            .await?;
        Ok(request_id)
    }

    async fn request_skip(&self, trusted_block: u64, target_block: u64) -> Result<String> {
        let trusted_header_hash = self
            .contract
            .block_height_to_header_hash(trusted_block)
            .await
            .unwrap();

        let input =
            SkipInputTuple::abi_encode_packed(&(trusted_block, trusted_header_hash, target_block));

        let skip_call = SkipCall {
            trusted_block,
            target_block,
        };
        let function_data = skip_call.encode();

        let request_id = self
            .client
            .submit_platform_request(
                self.config.chain_id,
                self.config.address,
                function_data.into(),
                self.config.skip_function_id,
                Bytes::copy_from_slice(&input),
            )
            .await?;
        Ok(request_id)
    }

    async fn request_skip_2(
        &self,
        trusted_hash: [u8; 32],
        trusted_block: u64,
        target_block: u64,
    ) -> Result<String> {
        let trusted_header_hash = trusted_hash;

        let input =
            SkipInputTuple::abi_encode_packed(&(trusted_block, trusted_header_hash, target_block));

        let skip_call = SkipCall {
            trusted_block,
            target_block,
        };
        let function_data = skip_call.encode();

        let request_id = self
            .client
            .submit_platform_request(
                self.config.chain_id,
                self.config.address,
                function_data.into(),
                self.config.skip_function_id,
                Bytes::copy_from_slice(&input),
            )
            .await?;
        Ok(request_id)
    }

    async fn is_consistent(&mut self, current_block: u64) {
        let expected_current_signed_header = self
            .data_fetcher
            .get_signed_header_from_number(current_block)
            .await;
        let expected_header = expected_current_signed_header.header.hash();
        let expected_header_bytes = expected_header.as_bytes();
        let contract_current_header = self
            .contract
            .block_height_to_header_hash(current_block)
            .await
            .unwrap();
        if expected_header_bytes != contract_current_header {
            panic!(
                "Current header in the contract does not match chain's header hash for block {:?}\n 
                From Tendermint RPC: {:?}\n 
                From contract: {:?}",
                current_block,
                String::from_utf8(hex::encode(expected_header)),
                String::from_utf8(hex::encode(contract_current_header))
            );
        }
    }

    async fn run(&mut self) {
        // Loop every 240 minutes.
        const LOOP_DELAY: u64 = 240;

        // The upper limit of the largest skip that can be requested. This is bounded by the unbonding
        // period, which for most Tendermint chains is ~2 weeks, or ~100K blocks with a block time
        // of 12s.
        let skip_max = self.contract.skip_max().await.unwrap();
        loop {
            let current_block = self.contract.latest_block().await.unwrap();

            // Consistency check for the headers (this should only happen if an invalid header,
            // typically the genesis header, is pushed to the contract). If this is triggered,
            // double check the genesis header in the contract.
            self.is_consistent(current_block).await;

            // Get the head of the chain.
            let latest_signed_header = self.data_fetcher.get_latest_signed_header().await;
            let latest_block = latest_signed_header.header.height.value();

            // Get the maximum block height we can request.
            let max_end_block = std::cmp::min(latest_block, current_block + skip_max);

            let target_block = self
                .data_fetcher
                .find_block_to_request(current_block, max_end_block)
                .await;
            println!("Current block: {}", current_block);
            println!("Target block: {}", target_block);

            if target_block - current_block == 1 {
                // Request the step if the target block is the next block.
                match self.request_step(current_block).await {
                    Ok(request_id) => {
                        info!("Step request submitted: {}", request_id)
                    }
                    Err(e) => {
                        error!("Step request failed: {}", e);
                        continue;
                    }
                };
            } else {
                // Request a skip if the target block is not the next block.
                match self.request_skip(current_block, target_block).await {
                    Ok(request_id) => {
                        info!("Skip request submitted: {}", request_id)
                    }
                    Err(e) => {
                        error!("Skip request failed: {}", e);
                        continue;
                    }
                };
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(60 * LOOP_DELAY)).await;
        }
    }

    async fn create_proof(
        &mut self,
        trusted_hash: [u8; 32],
        current_block_input: u64,
        target_block_input: u64,
    ) {
        if current_block_input >= target_block_input {
            error!("Invalid block input");
            error!("Current block: {}", current_block_input);
            error!("Target block: {}", target_block_input);
            error!("trusted_hash: {:?}", trusted_hash);
            return;
        }
        // The upper limit of the largest skip that can be requested. This is bounded by the unbonding
        // period, which for most Tendermint chains is ~2 weeks, or ~100K blocks with a block time
        // of 12s.
        // let skip_max = self.contract.skip_max().await.unwrap();
        // let current_block = self.contract.latest_block().await.unwrap();
        let current_block = current_block_input;

        // Consistency check for the headers (this should only happen if an invalid header,
        // typically the genesis header, is pushed to the contract). If this is triggered,
        // double check the genesis header in the contract.
        // self.is_consistent(current_block).await;

        // Get the head of the chain.
        // let latest_signed_header = self.data_fetcher.get_latest_signed_header().await;
        // let latest_block = latest_signed_header.header.height.value();

        // Get the maximum block height we can request.
        // let max_end_block = std::cmp::min(latest_block, current_block + skip_max);

        // let target_block = self
        //     .data_fetcher
        //     .find_block_to_request(current_block, max_end_block)
        //     .await;

        let target_block = target_block_input;

        println!("Current block: {}", current_block);
        println!("Target block: {}", target_block);

        // let target_block = target_block_input;
        println!("New target block: {}", target_block);

        // info!("request____start:{}request____end", "123123123");
        // return;

        if target_block - current_block == 1 {
            // Request the step if the target block is the next block.
            match self.request_step_2(trusted_hash, current_block).await {
                Ok(request_id) => {
                    info!("request____start{}request____end", request_id);
                    info!("Step request submitted: {}", request_id)
                }
                Err(e) => {
                    error!("Step request failed: {}", e);
                    return;
                }
            };
        } else {
            // Request a skip if the target block is not the next block.
            match self
                .request_skip_2(trusted_hash, current_block, target_block)
                .await
            {
                Ok(request_id) => {
                    info!("request____start{}request____end", request_id);
                    info!("Skip request submitted: {}", request_id)
                }
                Err(e) => {
                    error!("Skip request failed: {}", e);
                    return;
                }
            };
        }
    }
}

// #[tokio::main]
// async fn main() {
//     env::set_var("RUST_LOG", "info");
//     dotenv::dotenv().ok();
//     env_logger::init();

//     let mut operator = TendermintXOperator::new();
//     operator.run().await;
// }

#[tokio::main]
async fn main() {
    /*
        cargo run --package tendermintx --bin tendermintx --release 123456 654321
    */
    let arg = std::env::args().nth(3).expect("Expected a third argument");
    let args: Vec<String> = std::env::args().collect();
    let trusted = args[1].parse::<u64>().unwrap();
    let height = args[2].parse::<u64>().unwrap();
    let bytes = hex::decode(arg).expect("Invalid hex string");
    let mut array: [u8; 32] = [0; 32];
    array.copy_from_slice(&bytes);

    println!("Trusted block: {:?}", bytes);

    println!("Proof height: {}", height);
    env::set_var("RUST_LOG", "info");
    dotenv::dotenv().ok();
    env_logger::init();

    let mut operator = TendermintXOperator::new();
    operator.create_proof(array, trusted, height).await;
}
