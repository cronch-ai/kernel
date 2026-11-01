//! The WebAssembly ABI: the host allocates a request in the module's memory, calls, and frees the response.
//! The response's pointer and length come back packed in one `u64` (pointer high, length low).

fn into_raw(bytes: Box<[u8]>) -> (*mut u8, usize) {
    let len = bytes.len();
    (Box::into_raw(bytes) as *mut u8, len)
}

#[unsafe(no_mangle)]
pub extern "C" fn cronch_alloc(len: usize) -> *mut u8 {
    into_raw(vec![0u8; len].into_boxed_slice()).0
}

/// # Safety
/// `ptr` and `len` must come from `cronch_alloc` or `cronch_call`, and be freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cronch_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// # Safety
/// `ptr` and `len` must come from `cronch_alloc`; the request is consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cronch_call(ptr: *mut u8, len: usize) -> u64 {
    let request = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
    let (out, out_len) = into_raw(kernel::api::call(&request).into_boxed_slice());
    ((out as u64) << 32) | out_len as u64
}
