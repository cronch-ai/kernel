#[unsafe(no_mangle)]
pub extern "C" fn ping(n: u32) -> u32 {
    kernel::ping(n)
}
