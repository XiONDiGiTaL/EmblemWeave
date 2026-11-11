//! `std::__1::__hash_memory`, which is libc++'s 64-bit cityhash64.
//!
//! The title hashes strings during static initialisation, so this has to return
//! exactly what libc++ returns: a different value would change the hash tables
//! the game builds, not just its progress. The implementation below is a direct
//! port of `__murmur2_or_cityhash<size_t, 64>` from libc++'s
//! `__functional/hash.h`, with the reference vectors in the tests taken from a
//! real libc++ build (`std::hash<std::string>` on the same bytes).
//!
//! No shortcuts: every branch, rotation and prime is the original one.

const K0: u64 = 0xc3a5_c85c_97cb_3127;
const K1: u64 = 0xb492_b66f_be98_f273;
const K2: u64 = 0x9ae1_6a3b_2f90_404f;
const K3: u64 = 0xc949_d7c7_509e_6557;

fn load64(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

fn load32(bytes: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(word)
}

fn rotate(val: u64, shift: u32) -> u64 {
    if shift == 0 {
        val
    } else {
        (val >> shift) | (val << (64 - shift))
    }
}

/// Rotation for a shift that is always at least one, where `64 - shift` is
/// defined for every input.
fn rotate_by_at_least_1(val: u64, shift: u32) -> u64 {
    let s = shift & 63;
    (val >> s) | (val << (64 - s))
}

fn shift_mix(val: u64) -> u64 {
    val ^ (val >> 47)
}

fn hash_len_16(u: u64, v: u64) -> u64 {
    const MUL: u64 = 0x9ddf_ea08_eb38_2d69;
    let mut a = (u ^ v).wrapping_mul(MUL);
    a ^= a >> 47;
    let mut b = (v ^ a).wrapping_mul(MUL);
    b ^= b >> 47;
    b.wrapping_mul(MUL)
}

fn hash_len_0_to_16(s: &[u8]) -> u64 {
    let len = s.len() as u64;
    if len > 8 {
        let a = load64(s, 0);
        let b = load64(s, s.len() - 8);
        return hash_len_16(a, rotate_by_at_least_1(b.wrapping_add(len), len as u32)) ^ b;
    }
    if len >= 4 {
        let a = load32(s, 0);
        let b = load32(s, s.len() - 4);
        // `__a << 3` is a 32-bit shift in the original, then widened.
        return hash_len_16(len + (a.wrapping_shl(3) as u64), b as u64);
    }
    if !s.is_empty() {
        let a = s[0] as u32;
        let b = s[s.len() >> 1] as u32;
        let c = s[s.len() - 1] as u32;
        let y = a.wrapping_add(b << 8);
        let z = (len as u32).wrapping_add(c << 2);
        return shift_mix((y as u64).wrapping_mul(K2) ^ (z as u64).wrapping_mul(K3)).wrapping_mul(K2);
    }
    K2
}

fn hash_len_17_to_32(s: &[u8]) -> u64 {
    let len = s.len() as u64;
    let a = load64(s, 0).wrapping_mul(K1);
    let b = load64(s, 8);
    let c = load64(s, s.len() - 8).wrapping_mul(K2);
    let d = load64(s, s.len() - 16).wrapping_mul(K0);
    hash_len_16(
        rotate(a.wrapping_sub(b), 43)
            .wrapping_add(rotate(c, 30))
            .wrapping_add(d),
        a.wrapping_add(rotate(b ^ K3, 20))
            .wrapping_sub(c)
            .wrapping_add(len),
    )
}

fn weak_hash_len_32_with_seeds(w: u64, x: u64, y: u64, z: u64, a: u64, b: u64) -> (u64, u64) {
    let mut a = a.wrapping_add(w);
    let mut b = rotate(b.wrapping_add(a).wrapping_add(z), 21);
    let c = a;
    a = a.wrapping_add(x);
    a = a.wrapping_add(y);
    b = b.wrapping_add(rotate(a, 44));
    (a.wrapping_add(z), b.wrapping_add(c))
}

fn weak_hash_len_32(s: &[u8], a: u64, b: u64) -> (u64, u64) {
    weak_hash_len_32_with_seeds(
        load64(s, 0),
        load64(s, 8),
        load64(s, 16),
        load64(s, 24),
        a,
        b,
    )
}

fn hash_len_33_to_64(s: &[u8]) -> u64 {
    let len = s.len();
    let mut z = load64(s, 24);
    let mut a = load64(s, 0).wrapping_add((len as u64).wrapping_add(load64(s, len - 16)).wrapping_mul(K0));
    let mut b = rotate(a.wrapping_add(z), 52);
    let mut c = rotate(a, 37);
    a = a.wrapping_add(load64(s, 8));
    c = c.wrapping_add(rotate(a, 7));
    a = a.wrapping_add(load64(s, 16));
    let vf = a.wrapping_add(z);
    let vs = b.wrapping_add(rotate(a, 31)).wrapping_add(c);
    a = load64(s, 16).wrapping_add(load64(s, len - 32));
    z = z.wrapping_add(load64(s, len - 8));
    b = rotate(a.wrapping_add(z), 52);
    c = rotate(a, 37);
    a = a.wrapping_add(load64(s, len - 24));
    c = c.wrapping_add(rotate(a, 7));
    a = a.wrapping_add(load64(s, len - 16));
    let wf = a.wrapping_add(z);
    let ws = b.wrapping_add(rotate(a, 31)).wrapping_add(c);
    let r = shift_mix(
        vf.wrapping_add(ws)
            .wrapping_mul(K2)
            .wrapping_add(wf.wrapping_add(vs).wrapping_mul(K0)),
    );
    shift_mix(r.wrapping_mul(K0).wrapping_add(vs)).wrapping_mul(K2)
}

/// libc++ `std::__hash_memory`: cityhash64 over the bytes.
pub fn hash_memory(bytes: &[u8]) -> u64 {
    let len = bytes.len();
    if len <= 16 {
        return hash_len_0_to_16(bytes);
    }
    if len <= 32 {
        return hash_len_17_to_32(bytes);
    }
    if len <= 64 {
        return hash_len_33_to_64(bytes);
    }

    // Over 64 bytes: hash the end first, then walk 64-byte chunks keeping 56
    // bytes of state in v, w, x, y and z.
    let mut s = 0usize;
    let mut remaining = len as u64;
    let mut x = load64(bytes, len - 40);
    let mut y = load64(bytes, len - 16).wrapping_add(load64(bytes, len - 56));
    let mut z = hash_len_16(
        load64(bytes, len - 48).wrapping_add(len as u64),
        load64(bytes, len - 24),
    );
    let mut v = weak_hash_len_32(&bytes[len - 64..], len as u64, z);
    let mut w = weak_hash_len_32(&bytes[len - 32..], y.wrapping_add(K1), x);
    x = x.wrapping_mul(K1).wrapping_add(load64(bytes, 0));

    remaining = (remaining - 1) & !63u64;
    loop {
        x = rotate(
            x.wrapping_add(y)
                .wrapping_add(v.0)
                .wrapping_add(load64(bytes, s + 8)),
            37,
        )
        .wrapping_mul(K1);
        y = rotate(
            y.wrapping_add(v.1)
                .wrapping_add(load64(bytes, s + 48)),
            42,
        )
        .wrapping_mul(K1);
        x ^= w.1;
        y = y.wrapping_add(v.0).wrapping_add(load64(bytes, s + 40));
        z = rotate(z.wrapping_add(w.0), 33).wrapping_mul(K1);
        v = weak_hash_len_32(&bytes[s..], v.1.wrapping_mul(K1), x.wrapping_add(w.0));
        w = weak_hash_len_32(
            &bytes[s + 32..],
            z.wrapping_add(w.1),
            y.wrapping_add(load64(bytes, s + 16)),
        );
        std::mem::swap(&mut z, &mut x);
        s += 64;
        remaining -= 64;
        if remaining == 0 {
            break;
        }
    }
    hash_len_16(
        hash_len_16(v.0, w.0)
            .wrapping_add(shift_mix(y).wrapping_mul(K1))
            .wrapping_add(z),
        hash_len_16(v.1, w.1).wrapping_add(x),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values printed by `std::hash<std::string>` from a real libc++ build, one
    /// per length class the algorithm branches on.
    #[test]
    fn matches_libcxx_reference_vectors() {
        let cases: &[(&str, u64)] = &[
            ("", 11160318154034397263),
            ("a", 2603192927274642682),
            ("abc", 4220206313085259313),
            ("hello", 235368941344721064),
            ("__hash_memory", 11594394653592560679),
            ("The quick brown fox jumps over the lazy dog", 2788641486023529557),
            ("0123456789012345678901234567890123", 6751152006802866904),
            ("0123456789012345678901234567890123456789", 5006506542363679120),
        ];
        for (input, expected) in cases {
            assert_eq!(
                hash_memory(input.as_bytes()),
                *expected,
                "cityhash64 mismatch for {input:?} (len {})",
                input.len()
            );
        }
    }

    #[test]
    fn long_inputs_cross_the_chunk_loop() {
        // 128 bytes exercises the 64-byte loop twice.
        let mut bytes = Vec::new();
        for i in 0..128u8 {
            bytes.push(i.wrapping_mul(7));
        }
        let a = hash_memory(&bytes);
        let b = hash_memory(&bytes);
        assert_eq!(a, b, "hash must be deterministic");
        bytes[127] ^= 1;
        assert_ne!(a, hash_memory(&bytes), "a changed byte must change the hash");
    }
}
