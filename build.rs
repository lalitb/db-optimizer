fn main() {
    // Only compile protos if the cnpg feature is enabled
    if std::env::var("CARGO_FEATURE_CNPG").is_ok() {
        tonic_build::configure()
            .build_server(true)
            .compile(
                &[
                    "src/cnpg_plugin/proto/operator.proto",
                    "src/cnpg_plugin/proto/operator_lifecycle.proto"
                ],
                &["src/cnpg_plugin/proto"],
            )
            .unwrap();
    }
}
