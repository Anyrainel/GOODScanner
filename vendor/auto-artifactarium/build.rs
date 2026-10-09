fn main() {
    protobuf_codegen::Codegen::new()
        .pure()
        .cargo_out_dir("protos")
        .include("protos")
        .inputs(["protos/protos.proto"])
        .run_from_script();

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=protos/protos.proto");
}
