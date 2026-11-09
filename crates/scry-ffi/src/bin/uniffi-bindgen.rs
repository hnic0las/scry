//! Generates the Swift bindings: run by `scripts/build-ffi.sh`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
