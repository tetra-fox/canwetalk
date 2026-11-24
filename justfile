default:
    @just --list

# build the VST3 and CLAP bundles into target/bundled
bundle:
    cargo xtask bundle canwetalk --release

# the same for Windows hosts, into the same directory
bundle-windows:
    cargo xtask bundle canwetalk --release --target x86_64-pc-windows-gnu

# open the editor without a host, silent unless given a backend: `just editor --backend jack`
editor *args="--backend dummy":
    cargo run --features standalone --bin canwetalk -- {{ args }}

# open the example panel that is drawn with flat shapes alone, which needs no plugin
flat:
    cargo run -p controls-window --example flat

test:
    cargo test

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

# the gate for every push; ci.yml runs it
ci: lint test
