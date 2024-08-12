Run:

```bash
cargo stylus check
```

The error is:

```
Error: stylus checks failed

Caused by:
    program activation failed: failed to parse wasm

    Caused by:
        missing import pay_for_memory_grow

    Location:
        prover/src/programs/mod.rs:348:28

Location:
    /usr/local/cargo/registry/src/index.crates.io-6f17d22bba15001f/cargo-stylus-check-0.4.2/src/check.rs:171:9
```
