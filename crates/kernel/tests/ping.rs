use kernel::ping;

#[test]
fn ping_pongs() {
    assert_eq!(ping(1), 2);
    assert_eq!(ping(u32::MAX), 0);
}
