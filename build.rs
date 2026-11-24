#[allow(dead_code, unused_imports)]
#[path = "src/lib.rs"]
mod versolve;

use std::env;

fn main() {
    let target = env::var("TARGET").expect("TARGET is not set");
    println!("cargo:rustc-env=VERSOLVE_HOST={target}");
    let version = versolve::cargo::Resolver::new()
        .tag_prefix("v")
        .default_branch("master")
        .branch_env("VERSOLVE_BUILD_BRANCH")
        .resolve()
        .unwrap();
    println!("cargo:rustc-env=VERSOLVE_VERSION={version}");
}
