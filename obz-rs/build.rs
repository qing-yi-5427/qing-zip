fn main() {
    println!("cargo:rerun-if-changed=native/nana");
    if std::env::var("TARGET").as_deref() != Ok("x86_64-pc-windows-msvc") {
        return;
    }
    cc::Build::new()
        .cpp(true)
        .include("native/nana/vendor/SevenZip/CPP")
        .files([
            "native/nana/bridge.cpp",
            "native/nana/worker.cpp",
            "native/nana/process.cpp",
        ])
        .std("c++20")
        .flag("/EHsc")
        .flag("/guard:cf")
        .flag("/GS")
        .define("UNICODE", None)
        .define("_UNICODE", None)
        .define("NOMINMAX", None)
        .define("WIN32_LEAN_AND_MEAN", None)
        .compile("qing_selected_worker");
    for library in ["ole32", "oleaut32", "advapi32", "uuid"] {
        println!("cargo:rustc-link-lib={library}");
    }
}
