fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .compile(
            &["../scheduler/proto/allocation.proto"],
            &["../scheduler/proto"],
        )?;
    Ok(())
}
