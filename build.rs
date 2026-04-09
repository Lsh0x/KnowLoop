fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile Murmure gRPC proto
    let proto_path = "proto/murmure.proto";
    if std::path::Path::new(proto_path).exists() {
        tonic_build::compile_protos(proto_path)?;
    }
    Ok(())
}
