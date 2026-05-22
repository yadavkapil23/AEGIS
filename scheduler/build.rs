// Build script to compile proto files

use std::io::Result;

fn main() -> Result<()> {
    // Compile proto files
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/allocation.proto"], &["proto"])?;

    Ok(())
}
