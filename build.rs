fn main() {
    println!("cargo:rerun-if-changed=proto/udf.proto");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    println!("Protobufs will be compiled to: {}", out_dir);
    tonic_build::compile_protos("proto/udf.proto")
        .unwrap_or_else(|e| panic!("Failed to compile protos {:?}", e));
}
