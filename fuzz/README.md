# Fuzz Targets

The repository includes executable `cargo-fuzz` targets for the security-critical input surfaces:

- `domain_inputs` exercises domain canonicalization, URL validation, and payment-data redaction with arbitrary bytes;
- `rpc_frames` exercises strict versioned RPC deserialization with arbitrary bounded frames.

Run bounded local campaigns with:

```sh
cargo +nightly fuzz run domain_inputs -- -runs=10000
cargo +nightly fuzz run rpc_frames -- -runs=10000
```

CI compiles both harnesses and runs bounded corpora. Never upload an unsanitized crash reproducer because arbitrary fuzz input may resemble payment data.
