fn main() {
    if std::env::var_os("CARGO_FEATURE_UI").is_some() {
        slint_build::compile("ui/mullvad.slint").expect("Slint UI should compile");
    }

    let mut prost = prost_build::Config::new();
    prost.protoc_executable(
        protoc_bin_vendored::protoc_bin_path().expect("vendored protoc should be available"),
    );

    tonic_prost_build::configure()
        .with_extended_rust_types(true)
        .compile_with_config(
            prost,
            &[
                "proto/management_interface.proto",
                "proto/relay_selector.proto",
            ],
            &["proto"],
        )
        .expect("Mullvad daemon protocol should compile");

    println!("cargo:rerun-if-changed=proto/management_interface.proto");
    println!("cargo:rerun-if-changed=proto/relay_selector.proto");
    println!("cargo:rerun-if-changed=ui/mullvad.slint");
}
