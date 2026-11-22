#[allow(dead_code, unused_imports)]
#[path = "src/lib.rs"]
mod versolve;

use std::env;

fn main() {
    let target = env::var("TARGET").expect("TARGET is not set");
    println!("cargo:rustc-env=VERSOLVE_HOST={target}");
    versolve::build::Config::new()
        .tag_prefix("v")
        .default_branch("master")
        .branch_env("VERSOLVE_BUILD_BRANCH")
        .emit("VERSOLVE_VERSION")
        .unwrap();
}
