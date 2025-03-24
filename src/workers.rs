use crate::{ServiceStatus, state::ApplicationState, storage::SpotEntryEvent};
use starknet::{
    core::{
        types::{BlockId, EventFilter, Felt, MaybePendingBlockWithTxHashes},
        utils::starknet_keccak,
    },
    providers::{
        Provider, Url,
        jsonrpc::{HttpTransport, JsonRpcClient},
    },
};
use std::sync::Arc;

use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

const BLOCKS_IN_1_HOUR: u8 = 120;
const EVENT_CHUNK_SIZE: u64 = 1000;
const JSON_RPC_POLL_TIMEOUT: u64 = 15000;
const ONE_HOUR: Duration = Duration::from_secs(3600);

/// This worker connects to Starknet node using JSON-RPC and queries for events from Pragma price oracle and send
/// batches to the channel it get as argument.
///
/// # Errors
///
/// This function will return an error if:
/// - JSON RPC url is invalid.
/// - In case of any RPC errors
/// - If publishing channel is closed.
async fn fetch_events(tx: UnboundedSender<Vec<SpotEntryEvent>>) -> Result<(), String> {
    let starknet_sepolia_url: Url = Url::parse("https://starknet-sepolia.public.blastapi.io/rpc/v0_7")
        .map_err(|_| "Fetcher can't parse Node Url")?;
    let provider = JsonRpcClient::new(HttpTransport::new(starknet_sepolia_url));

    let btc_usd_pair_id: Felt = Felt::from_bytes_be_slice("BTC/USD".as_bytes());
    let oracle_contract_address =
        Some(Felt::from_hex_unchecked("0x36031daa264c24520b11d93af622c848b2499b66b41d611bac95e13cfca131a"));
    let submitted_spot_entry_event_keys = vec![vec![starknet_keccak("SubmittedSpotEntry".as_bytes())]];

    // Initial scanning parameters, we take latest finalised block and start 120 blocks before (30s per block is needed
    // for production)
    let mut to_block_number = provider.block_number().await.map_err(|_| "Can't fetch latest block number")?;

    let mut from_block_number = to_block_number - u64::from(BLOCKS_IN_1_HOUR);

    let block = provider
        .get_block_with_tx_hashes(BlockId::Number(from_block_number))
        .await
        .map_err(|_| "Can't get block with_tx_hashes")?;

    if let MaybePendingBlockWithTxHashes::Block(block) = block {
        let time_diff = SystemTime::now()
            .checked_sub(Duration::from_secs(block.timestamp))
            .ok_or("Can't calculate diff between current and block.timestamp")?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| "Can't calculate duration for till first block")?
            .as_secs();

        println!("Starting at block: {from_block_number:#?} with timestamp {time_diff:#?}s ago");
    }

    let mut continuation_token = None;
    loop {
        let filter = EventFilter {
            address: oracle_contract_address,
            keys: Some(submitted_spot_entry_event_keys.clone()),
            from_block: Some(BlockId::Number(from_block_number)),
            to_block: Some(BlockId::Number(to_block_number)),
        };

        if from_block_number == to_block_number {
            tokio::time::sleep(Duration::from_millis(JSON_RPC_POLL_TIMEOUT)).await;
            to_block_number = provider.block_number().await.map_err(|_| "Can't fetch latest block number")?;
        }

        let event_page = provider
            .get_events(filter.clone(), continuation_token, EVENT_CHUNK_SIZE)
            .await
            .map_err(|_| "Can't fetch events")?;

        let events: Vec<SpotEntryEvent> = event_page
            .events
            .iter()
            .map(|event| SpotEntryEvent::try_from(event.data.as_slice()))
            .filter_map(|res| res.ok())
            .filter(|event| event.pair_id == btc_usd_pair_id)
            .collect();

        tx.send(events).map_err(|_| "Can't publish events to channel")?;

        continuation_token = event_page.continuation_token;

        if continuation_token.is_none() {
            // advance blocks
            from_block_number = to_block_number;
        }
    }
}

/// This worker receives events in batches store them into storage and trigger twap recalculations.
///
/// # Panics
///
/// Panics if can't acqure storage write lock.
///
/// # Errors
///
/// This function will return an error if datetime calculations failed
async fn process_events(
    state: Arc<ApplicationState>,
    mut rx: UnboundedReceiver<Vec<SpotEntryEvent>>,
) -> Result<(), String> {
    loop {
        let hour_ago = SystemTime::now().checked_sub(ONE_HOUR).ok_or("Can't calculate now - hour")?;

        let duration_since_hour_ago =
            hour_ago.duration_since(SystemTime::UNIX_EPOCH).map_err(|_| "Can't calculate duration")?;

        if let Some(events) = rx.recv().await {
            // Storage changes in that block
            let mut storage = state.storage.write().unwrap();
            for event in events {
                storage.append(event);
            }
            storage.clean_older_than(duration_since_hour_ago.as_secs());
            storage.calculate_and_sign_twap(state.secret_key);
        }
    }
}

pub trait WorkerRunner {
    async fn start_fetcher(self, tx: UnboundedSender<Vec<SpotEntryEvent>>) -> Result<(), String>;
    async fn start_processor(self, rx: UnboundedReceiver<Vec<SpotEntryEvent>>) -> Result<(), String>;
}

impl WorkerRunner for Arc<ApplicationState> {
    async fn start_fetcher(self, tx: UnboundedSender<Vec<SpotEntryEvent>>) -> Result<(), String> {
        let result = fetch_events(tx).await;

        if let Err(message) = result {
            *self.fetcher_status.write().unwrap() = ServiceStatus::Failed { message: message.to_string() };
        } else {
            *self.fetcher_status.write().unwrap() = ServiceStatus::Failed { message: "Unknown reason".to_string() };
        };

        Ok(())
    }

    async fn start_processor(self, rx: UnboundedReceiver<Vec<SpotEntryEvent>>) -> Result<(), String> {
        let result = process_events(self.clone(), rx).await;

        if let Err(message) = result {
            *self.processor_status.write().unwrap() = ServiceStatus::Failed { message: message.to_string() };
        } else {
            *self.processor_status.write().unwrap() = ServiceStatus::Failed { message: "Unknown reason".to_string() };
        };

        Ok(())
    }
}
