fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=migrations");
    tonic_build::configure()
        .build_client(false)
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile(&["proto/auth.proto"], &["proto/"])?;
    Ok(())
}
