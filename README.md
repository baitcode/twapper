# Prerequisites

These instructions do not support Windows.

To run the application you'll need rust installed. Check that [link](https://rustup.rs/) to install [rust](https://rustup.rs/)

# Run

To quickly run the server use:

```bash

cargo run

```

by default server is attached to port 3000 on localhost. This is not configurable.
Upon running server generates private and public keys for data signing. Those are stored in memory and are not persisted.

# Test

```bash

cargo test

```

# Build

```bash

cargo build

```

# API

## /health

This endpoint checks if event fetching worker and event processing worker are active and working. If everything is ok the response is:

STATUS CODE: 200
```json
{
  "Ok": null
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

`twamp` is U256 integer, that was converted to big endian byte array and encoded as lower case hex string. 
`signature` is hex encoded ECDSA signature conveted to byte array using compact raw format (Concatenated `r` and `s` values) without recovery id.
`pk` is hex encoded public key bytes in compressed format. This is a ECDSA public key from secp256k1 curve.

To check signature one would need to convert twamp hex value to big endian style byte array, use it as an input to sha256 hash function to generate digest, and then verify that digest using Public Key and Signature values. The curve used for verification is secp256k1.