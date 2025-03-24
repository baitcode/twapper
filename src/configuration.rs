use crate::storage::SpotEntryStorage;

use secp256k1::{
    Message, PublicKey, Secp256k1, SecretKey,
    constants::PUBLIC_KEY_SIZE,
    hashes::{Hash, hex::FromHex, sha256},
    rand::rngs::OsRng,
};
use std::{env, sync::RwLock};

pub enum ServiceStatus {
    Running,
    Failed { message: String },
}

pub struct ApplicationConfiguration {
    pub port: u32,
    pub host: String,

    pub secret_key: SecretKey,
    pub public_key: PublicKey,

    pub storage: RwLock<SpotEntryStorage>,

    pub fetcher_status: RwLock<ServiceStatus>,
    pub processor_status: RwLock<ServiceStatus>,
}

impl ApplicationConfiguration {
    pub fn new() -> Result<ApplicationConfiguration, String> {
        let secp: Secp256k1<secp256k1::All> = Secp256k1::gen_new();

        let port: u32 = if let Ok(key) = env::var("PORT") {
            key.parse().map_err(|_| "Value in PORT variable is invalid")?
        } else {
            3000_u32
        };

        let host: String = if let Ok(key) = env::var("host") { key } else { "0.0.0.0".to_string() };

        let secret_key = if let Ok(key) = env::var("SECRET_KEY") {
            let secret_bytes = <[u8; 32]>::from_hex(key.as_str()).map_err(|_| "Invalid env var SECRET_KEY")?;

            SecretKey::from_byte_array(&secret_bytes).map_err(|_| "Secret key format invalid")?
        } else {
            let (secret_key, _) = secp.generate_keypair(&mut OsRng);
            secret_key
        };

        let public_key = if let Ok(key) = env::var("PUBLIC_KEY") {
            let public_bytes =
                <[u8; PUBLIC_KEY_SIZE]>::from_hex(key.as_str()).map_err(|_| "Invalid env var PUBLIC_KEY")?;

            PublicKey::from_byte_array_compressed(&public_bytes).map_err(|_| "Public key format invalid")?
        } else {
            PublicKey::from_secret_key(&secp, &secret_key)
        };

        let digest = sha256::Hash::hash([0_u8, 0_u8, 0_u8, 0_u8].as_slice());
        let message = Message::from_digest(digest.to_byte_array());
        let signature = secp.sign_ecdsa(&message, &secret_key);

        secp.verify_ecdsa(&message, &signature, &public_key).map_err(|_| "Public and Secret keys do not match.")?;

        Ok(ApplicationConfiguration {
            host,
            port,
            secret_key,
            public_key,
            storage: RwLock::new(SpotEntryStorage::new()),
            fetcher_status: RwLock::new(ServiceStatus::Running),
            processor_status: RwLock::new(ServiceStatus::Running),
        })
    }
}
