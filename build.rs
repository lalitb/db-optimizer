fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Rebuild when proto files change
    println!("cargo:rerun-if-changed=src/cnpg_plugin/proto/operator.proto");
    println!("cargo:rerun-if-changed=src/cnpg_plugin/proto/operator_lifecycle.proto");
    println!("cargo:rerun-if-changed=src/cnpg_plugin/proto");

    // Only generate CNPG gRPC code when the feature is enabled.
    // This keeps non-CNPG builds fast and avoids requiring protoc for all builds.
    if std::env::var_os("CARGO_FEATURE_CNPG").is_none() {
        return Ok(());
    }

    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile(
            &[
                "src/cnpg_plugin/proto/operator.proto",
                "src/cnpg_plugin/proto/operator_lifecycle.proto",
            ],
            &["src/cnpg_plugin/proto"],
        )?;

    Ok(())
}
