#[cfg(not(target_os = "android"))]
fn main() {
    matterweave_monsters_app::run_desktop();
}
#[cfg(target_os = "android")]
fn main() {}
