mod storage;

use storage::{SpotEntryEvent, SpotEntryStorage};

use secp256k1::{PublicKey, Secp256k1, SecretKey, hashes::hex::DisplayHex, rand::rngs::OsRng};
use serde::Serialize;
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
use std::{
    ops::Deref,
    sync::{Arc, RwLock},
};

use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header::CONTENT_TYPE},
    response::{AppendHeaders, IntoResponse},
    routing::get,
};

const BLOCKS_IN_1_HOUR: u8 = 120;
const EVENT_CHUNK_SIZE: u64 = 1000;
const JSON_RPC_POLL_TIMEOUT: u64 = 15000;
const ONE_HOUR: Duration = Duration::from_secs(3600);

async fn fetch_events(tx: UnboundedSender<Vec<SpotEntryEvent>>) -> Result<(), String> {
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse("https://starknet-sepolia.public.blastapi.io/rpc/v0_7").unwrap(),
    ));

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

        let events = event_page
            .events
            .iter()
            .map(|event| SpotEntryEvent::from_event_data(&event.data))
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

async fn process_events(
    state: Arc<ApplicationState>,
    mut rx: UnboundedReceiver<Vec<SpotEntryEvent>>,
) -> Result<(), String> {
    loop {
        let hour_ago = SystemTime::now().checked_sub(ONE_HOUR).ok_or("Can't calculate now - hour")?;

        let duration_since_hour_ago =
            hour_ago.duration_since(SystemTime::UNIX_EPOCH).map_err(|_| "Can't calcualte duration")?;

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

enum ServiceStatus {
    Running,
    Failed { message: String },
}

struct ApplicationState {
    secret_key: SecretKey,
    public_key: PublicKey,

    storage: RwLock<SpotEntryStorage>,

    fetcher_status: RwLock<ServiceStatus>,
    processor_status: RwLock<ServiceStatus>,
}

impl ApplicationState {
    fn new() -> ApplicationState {
        let secp: Secp256k1<secp256k1::All> = Secp256k1::gen_new();
        let (secret_key, public_key) = secp.generate_keypair(&mut OsRng);

        ApplicationState {
            secret_key,
            public_key,
            storage: RwLock::new(SpotEntryStorage::new()),
            fetcher_status: RwLock::new(ServiceStatus::Running),
            processor_status: RwLock::new(ServiceStatus::Running),
        }
    }
}

trait Application {
    async fn start_fetcher(self, tx: UnboundedSender<Vec<SpotEntryEvent>>) -> Result<(), String>;
    async fn start_processor(self, rx: UnboundedReceiver<Vec<SpotEntryEvent>>) -> Result<(), String>;
}

impl Application for Arc<ApplicationState> {
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

// TODO: add proper serialiser
#[derive(Serialize)]
struct Data {
    twap: String,
    signature: String,
    pk: String,
}

async fn data_handler(State(state): State<Arc<ApplicationState>>) -> impl IntoResponse {
    let storage = { state.storage.read().unwrap() };

    let twap = if let Some(value) = storage.twap {
        value
    } else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err("Data not ready".to_string())),
        );
    };

    let signature = if let Some(value) = storage.signature {
        value
    } else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err("Data not ready".to_string())),
        );
    };

    let twap_bytes = [twap.high().to_be_bytes(), twap.low().to_be_bytes()].concat();
    let twap_serialised = twap_bytes.to_lower_hex_string();

    let signature = signature.serialize_compact().to_lower_hex_string();

    (
        StatusCode::OK,
        AppendHeaders([(CONTENT_TYPE, "application/json")]),
        Json(Result::Ok(Data { twap: twap_serialised, signature, pk: state.public_key.to_string() })),
    )
}

async fn health_handler(State(state): State<Arc<ApplicationState>>) -> impl IntoResponse {
    if let ServiceStatus::Failed { message } = state.fetcher_status.read().unwrap().deref() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err(message.to_string())),
        );
    }

    if let ServiceStatus::Failed { message } = state.processor_status.read().unwrap().deref() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err(message.to_string())),
        );
    }

    (StatusCode::OK, AppendHeaders([(CONTENT_TYPE, "application/json")]), Json(Result::Ok("good".to_string())))
}

#[tokio::main]
async fn main() {
    let app_state = Arc::new(ApplicationState::new());

    let app = Router::new()
        .route("/data", get(data_handler))
        .route("/health", get(health_handler))
        .with_state(app_state.clone());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    let (tx, rx) = mpsc::unbounded_channel::<Vec<SpotEntryEvent>>();

    let fetching_handle = tokio::spawn(app_state.clone().start_fetcher(tx));
    let processing_handle = tokio::spawn(app_state.clone().start_processor(rx));

    if let Err(z) = axum::serve(listener, app).await {
        panic!("{z}");
    };

    fetching_handle.abort();
    processing_handle.abort();
}
