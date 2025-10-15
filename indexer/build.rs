fn main() {
    prost_build::compile_protos(&["src/alkanes/protorune.proto"], &["src/alkanes/"]).unwrap();
}
