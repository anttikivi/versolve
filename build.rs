#[allow(dead_code, unused_imports)]
#[path = "src/lib.rs"]
mod versolve;

fn main() {
    versolve::build::Config::new()
        .tag_prefix("v")
        .default_branch("master")
        .branch_env("VERSOLVE_BUILD_BRANCH")
        .emit("VERSOLVE_VERSION")
        .unwrap();
}
