#[cfg(not(target_os = "android"))]
fn main() {
    matterweave_explorer::run_desktop();
}
#[cfg(target_os = "android")]
fn main() {}
