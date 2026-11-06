# safesh (Rust)

A non-interactive bash runner for AI coding agents. It parses bash with [brush](https://github.com/reubeno/brush), runs commands directly, and checks every command and redirect against a permission policy before it happens.

This is the Rust rewrite of [safesh](https://github.com/jchip/safesh) (Deno/TypeScript). It's a work in progress. See `notes/README.md` for the design and status.

```sh
cargo build
cargo test --workspace
```
