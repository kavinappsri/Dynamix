fn main() {
    println!("cargo:rerun-if-changed=aarch64/aarch64_linker.ld");
    println!("cargo::rerun-if-changed=build.rs");
}
