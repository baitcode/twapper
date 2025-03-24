# Prerequisites

These instructions do not support Windows.

To run the application you'll need rust installed. Check that [link](https://rustup.rs/) to install [rust](https://rustup.rs/)

# Run

To quickly run the server use:

```bash

cargo run

```

by default server is attached to port 3000 on localhost. This is not configurable.

If you want to supply your own public key use `PUBLIC_KEY` enviroment variable. Value should be lower hex encoded public key bytes in compressed form (33 bytes).
If you want to supply your own secret key use `SECRET_KEY` enviroment variable. Value should be lower hex encoded secret key bytes. (32 bytes)

NOTE: If you supply `SECRET_KEY` only, public key will be derived from it.

WARNING: If you supply `PUBLIC_KEY` only, key validation will fail, as servere will generate new `SECRET_KEY` and it's highly unlikely that those would match.

Here are keypairs for testing: 

```
PUBLIC_KEY="0252dd2b8b729ab74497c172887b4cc56b427dcf0bf0368a3f93b5ff79b3f09410"
SECRET_KEY="d150f1224d8c75c25f186d0d18c058201a4f6e9ca13237ade9eb9988ef391de5"
```

```
PUBLIC_KEY="02b74d0beb364934725776ff37f7c8839e772bfe28089709fa3c6f33debda9df02"
SECRET_KEY="05981a8e771720be8d9fbbe0937d4809304a3450a8e9ef2c494af81753f79ca9"
```

# Test

```bash
cargo test
```

# Build

```bash
cargo build
```

# Module structure

`storage.rs`: 

has code connected to storing events, twamp calculation and data signing.

`main.rs`: 

has fetch_events worker, that connects to JSON RPC and fetches SubmittedSpotEntry events for recent 120 blocks (roughtly ONE hour). Filters out all pairs except for BTC/USD and passes batch to processor.

has process_events worker, that takes recent batch of event ads them to storage, cleans storage up then calculates twapm and generates signature.

has definitions for axum server with `data` and `health` headers.

# API

## /health

This endpoint checks if event fetching worker and event processing worker are active and working. If everything is ok the response is:

STATUS CODE: 200
```json
{
  "Ok": "Good"
}
```

If something is wrong the response is:

STATUS CODE: 500
```json
{
  "Err": "MESSAGE"
}
```

## /data

This endpoint returns currently calculated twapm data along with signature and public key. If data is not ready the response would be:

STATUS CODE: 500
```json
{
  "Err": "MESSAGE"
}
```

If everything is ok then the response would be:

STATUS CODE: 200
```json
{
    "Ok": {
        "twap": "0000000000000000000000000000000000000000000000000000079c7402dfd3",
        "signature":"d84d47ddb8483e5cab68d9269bdd75b47eb556c194eb2378998f752c8f6908ff5a11a7ec12414f8652c984614bf56ffec7996bd4924c29b8834e236b16ecc75f",
        "pk":"023946664473fcf226abc6d9fc094fca7eb4795cff340064e285ea3689fda420a2"
    }
}
```

`twamp` is an encoded `Fixed Point` value. Value is represented by bytes in a big endian fashion. This value should always be less than 256 bits long, first 192 bit is reserved for quotient, last 64 bits for remainder. Bytes are encoded using lowercase hex encoding. Internally it is represented by `Big Integer` type.

`signature` is hex encoded ECDSA signature conveted to byte array using compact raw format (Concatenated `r` and `s` values) without recovery id.

`pk` is hex encoded public key bytes in compressed format. This is a ECDSA public key from secp256k1 curve.

To check signature one would need to convert twamp hex value to big endian style byte array, use it as an input to sha256 hash function to generate digest, and then verify that digest using Public Key and Signature values. The curve used for verification is secp256k1.