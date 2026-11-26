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

test:
    cargo test

# draw the editor to a png at twice its size, after a voice has played through the plugin with
# formant follow on: `just picture out.png`. needs a graphics device
picture file:
    CANWETALK_PICTURE="{{ file }}" cargo test the_editor_draws_what_the_plugin_hears -- --ignored

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

# the gate for every push; ci.yml runs it
ci: lint test
