/** @param {BufferSource} bytes */
export async function instantiate(bytes) {
  const { instance } = await WebAssembly.instantiate(bytes, {});
  return { ping: (n) => instance.exports.ping(n) >>> 0 };
}
