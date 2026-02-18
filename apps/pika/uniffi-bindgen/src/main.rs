fn main() {
    // Workaround for non-nightly cargo: provide a local bindgen binary.
    uniffi::uniffi_bindgen_main()
}
