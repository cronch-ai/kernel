export interface Kernel {
  ping(n: number): number;
}

export function instantiate(bytes: BufferSource): Promise<Kernel>;
