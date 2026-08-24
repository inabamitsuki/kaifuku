fn main() {
    println!("cargo:rerun-if-changed=config.h");
    println!("cargo:rerun-if-changed=c_src/");
    println!("cargo:rerun-if-changed=Makefile.photorec");

    let status = std::process::Command::new("make")
        .args(&["-f", "Makefile.photorec", "libphotorec.a"])
        .status()
        .expect("Failed to run make");

    if !status.success() {
        panic!("C compilation failed");
    }

    let mut build = cc::Build::new();
    build
        .include("c_src/jpegrepair")
        .file("c_src/jpegrepair/jpegrepair_mem.c")
        .file("c_src/jpegrepair/transupp.c")
        .warnings(false)
        .compile("jpegrepair");
    println!("cargo:rerun-if-changed=c_src/jpegrepair/");

    println!("cargo:rustc-link-search=.");
    println!("cargo:rustc-link-lib=static=photorec");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=z");
    println!("cargo:rustc-link-lib=jpeg");
}
