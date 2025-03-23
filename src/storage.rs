use num_bigint::BigUint;
use secp256k1::{
    Message, Secp256k1, SecretKey,
    ecdsa::Signature,
    hashes::{Hash, sha256},
};
use starknet::core::types::Felt;
use std::{collections::HashMap, fmt::Debug};

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct SpotEntryEvent {
    timestamp: u64,
    price: u128,
    pub pair_id: Felt,
}

impl TryFrom<&[Felt]> for SpotEntryEvent {
    type Error = String;

    fn try_from(value: &[Felt]) -> Result<Self, Self::Error> {
        let timestamp = value[0].try_into().map_err(|_| "Can't convert timestamp for event")?;
        let price = value[3].try_into().map_err(|_| "Can't convert price for event")?;
        Ok(SpotEntryEvent { timestamp, price, pair_id: value[4] })
    }
}

impl PartialOrd for SpotEntryEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.timestamp.cmp(&other.timestamp))
    }
}

impl Ord for SpotEntryEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}

pub struct SpotEntryStorage {
    secp: Secp256k1<secp256k1::All>,
    data: HashMap<u64, SpotEntryEvent>,
    pub twap: Option<BigUint>,
    pub signature: Option<Signature>,
}

impl SpotEntryStorage {
    pub fn new() -> SpotEntryStorage {
        SpotEntryStorage { secp: Secp256k1::gen_new(), data: HashMap::with_capacity(7200), twap: None, signature: None }
    }

    pub fn append(&mut self, event: SpotEntryEvent) {
        // Events can have same timestamp. Should be an aggregated value. Say mean.
        self.data.insert(event.timestamp, event);
    }

    pub fn clean_older_than(&mut self, timestamp: u64) {
        let keys: Vec<u64> = self.data.keys().filter(|k| **k <= timestamp).cloned().collect();

        for key in keys {
            self.data.remove(&key);
        }
    }

    pub fn calculate_and_sign_twap(&mut self, secret_key: SecretKey) {
        let mut events: Vec<&SpotEntryEvent> = self.data.values().collect();
        events.sort_by_key(|e| e.timestamp);

        let mut last_timestamp = 0_u64;
        let mut numenator_aggregate = BigUint::from(0_u128);
        let mut divisor_aggregate = 0_u64;

        for event in events {
            if last_timestamp == 0 {
                last_timestamp = event.timestamp;
                continue;
            }

            let timedelta = event.timestamp - last_timestamp;
            last_timestamp = event.timestamp;

            numenator_aggregate += event.price * u128::from(timedelta);
            divisor_aggregate += timedelta;
        }

        if divisor_aggregate == 0 {
            return;
        }

        let twap: BigUint = (numenator_aggregate << 64) / divisor_aggregate;
        let twap_bytes = twap.to_bytes_be();
        self.twap = Some(twap);

        let digest = sha256::Hash::hash(twap_bytes.as_slice());
        let message = Message::from_digest(digest.to_byte_array());
        self.signature = Some(self.secp.sign_ecdsa(&message, &secret_key));
    }
}

#[cfg(test)]
mod test {
    use rand::prelude::*;
    use std::time::{Duration, SystemTime};

    use super::*;
    use secp256k1::rand::rngs::OsRng;

    #[test]
    fn storage_ields_initialization() {
        let mut storage = SpotEntryStorage::new();
        let (secret_key, _) = storage.secp.generate_keypair(&mut OsRng);

        assert_eq!(storage.signature, None);
        assert_eq!(storage.twap, None);
        assert_eq!(storage.data.len(), 0);

        storage.calculate_and_sign_twap(secret_key);

        assert_eq!(storage.signature, None);
        assert_eq!(storage.twap, None);
        assert_eq!(storage.data.len(), 0);
    }

    #[test]
    fn simple_event_addition() {
        let mut storage = SpotEntryStorage::new();
        let event_factory = |timestamp, price| SpotEntryEvent { timestamp, price, pair_id: Felt::ZERO };

        for i in 0..10000 {
            let ts = SystemTime::now()
                .checked_sub(Duration::from_secs(i))
                .unwrap()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();

            let event = event_factory(ts.as_secs(), 100_u64.into());

            storage.append(event);
        }

        assert_eq!(storage.data.len(), 10000);
    }

    #[test]
    fn event_cleaning() {
        let mut storage = SpotEntryStorage::new();
        let event_factory = |timestamp, price| SpotEntryEvent { timestamp, price, pair_id: Felt::ZERO };

        for i in 0..10000 {
            let ts = SystemTime::now()
                .checked_sub(Duration::from_secs(i))
                .unwrap()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();

            let event = event_factory(ts.as_secs(), 100_u64.into());

            storage.append(event);
        }

        assert_eq!(storage.data.len(), 10000);

        let hour_ago = SystemTime::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();

        storage.clean_older_than(hour_ago.as_secs());

        assert_eq!(storage.data.len(), 3600);
    }

    #[test]
    fn events_on_same_ts_overwrite_each_other() {
        let mut storage = SpotEntryStorage::new();
        let event_factory = |timestamp, price| SpotEntryEvent { timestamp, price, pair_id: Felt::ZERO };

        for _ in 0..3 {
            for i in 0..100 {
                let ts = SystemTime::now()
                    .checked_sub(Duration::from_secs(i))
                    .unwrap()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap();

                let event = event_factory(ts.as_secs(), 100_u64.into());

                storage.append(event);
            }
        }

        assert_eq!(storage.data.len(), 100);
    }

    #[test]
    fn test_naive_twap_calculation() {
        let mut storage = SpotEntryStorage::new();
        let event_factory = |timestamp, price| SpotEntryEvent { timestamp, price, pair_id: Felt::ZERO };

        for i in 0..100 {
            let ts = SystemTime::now()
                .checked_sub(Duration::from_secs(i))
                .unwrap()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();

            let event = event_factory(ts.as_secs(), 100_u128);

            storage.append(event);
        }

        assert_eq!(storage.data.len(), 100);

        let (secret_key, _) = storage.secp.generate_keypair(&mut OsRng);
        storage.calculate_and_sign_twap(secret_key);

        assert!(storage.twap.is_some());
        assert_eq!(storage.twap.unwrap() >> 64, BigUint::from(100_u64));
    }

    #[test]
    fn test_complex_twap_calculation() {
        let mut storage = SpotEntryStorage::new();
        let event_factory = |timestamp, price| SpotEntryEvent { timestamp, price, pair_id: Felt::ZERO };

        let mut ts = SystemTime::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();

        let event = event_factory(ts.as_secs(), 100_u64.into());
        storage.append(event);

        let mut rng = rand::rng();

        let mut numenator_aggregate = 0_u128;
        let mut divisor_aggregate = 0_u128;

        for _ in 0..99 {
            let timedelta = rng.random::<u8>() / 2 + 10;
            ts = ts.checked_add(Duration::from_secs(timedelta.into())).unwrap();

            let price: u32 = rng.random::<u32>();
            let event = event_factory(ts.as_secs(), price.into());

            storage.append(event);
            numenator_aggregate += u128::from(timedelta) * u128::from(price);
            divisor_aggregate += u128::from(timedelta);
        }

        let twap = numenator_aggregate / divisor_aggregate;

        assert_eq!(storage.data.len(), 100);

        let (secret_key, _) = storage.secp.generate_keypair(&mut OsRng);
        storage.calculate_and_sign_twap(secret_key);

        assert!(storage.twap.is_some());
        assert_eq!(storage.twap.unwrap() >> 64, BigUint::from(twap));
    }
}
