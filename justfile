default:
    @just --list

# build the VST3 and CLAP bundles into target/bundled
bundle:
    cargo xtask bundle canwetalk --release

# the same for Windows hosts, into the same directory
bundle-windows:
    cargo xtask bundle canwetalk --release --target x86_64-pc-windows-gnu

test:
    cargo test

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

# the gate for every push; ci.yml runs it
ci: lint test
