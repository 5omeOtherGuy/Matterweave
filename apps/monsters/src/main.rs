#[cfg(not(target_os = "android"))]
fn main() {
    println!("Mossbound desktop launcher pending");
}
#[cfg(target_os = "android")]
fn main() {}
