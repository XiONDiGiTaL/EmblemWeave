//! NVN render-state objects: blend, color, channel mask, depth/stencil,
//! multisample, polygon, vertex attribute and vertex stream state.
//!
//! These are small structures in guest memory that the game sets, copies by
//! value and binds by pointer. Their state therefore lives in the structure's
//! own bytes: each setter packs its arguments into fixed bit fields of the
//! structure, each getter unpacks them, and a bind copies the bytes. The
//! packing is this host's own (the game never reads the bytes itself); each
//! layout fits inside the structure size NVN reserves for that object.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use crate::nvn::called;
use ew_lift::State;

/// A field: which argument holds it, its bit offset and width.
#[derive(Clone, Copy)]
struct F {
    arg: usize,
    at: u32,
    bits: u32,
}

const fn f(arg: usize, at: u32, bits: u32) -> F {
    F { arg, at, bits }
}

/// How a method maps onto the packed word.
struct Method {
    name: &'static str,
    fields: &'static [F],
    /// For per-target or per-face methods: the argument holding the index or
    /// face mask, and the bit stride between consecutive targets.
    indexed: Option<(usize, u32, Indexing)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Indexing {
    /// A render target number.
    Target,
    /// An NVN face mask: front = 1, back = 2.
    Face,
}

struct Kind {
    prefix: &'static str,
    /// Bytes NVN reserves for the structure; the packed state uses at most 8.
    size: u64,
    defaults: u64,
    methods: &'static [Method],
}

const fn m(name: &'static str, fields: &'static [F]) -> Method {
    Method { name, fields, indexed: None }
}

const KINDS: &[Kind] = &[
    Kind {
        prefix: "nvnPolygonState",
        size: 4,
        // cull none, counter-clockwise front, fill, no offset
        defaults: (1 << 2) | (2 << 3),
        methods: &[
            m("CullFace", &[f(1, 0, 2)]),
            m("FrontFace", &[f(1, 2, 1)]),
            m("PolygonMode", &[f(1, 3, 2)]),
            m("PolygonOffsetEnables", &[f(1, 5, 3)]),
        ],
    },
    Kind {
        prefix: "nvnColorState",
        size: 4,
        // blending off, logic op copy, alpha test always
        defaults: (3 << 8) | (8 << 12),
        methods: &[
            Method { name: "BlendEnable", fields: &[f(2, 0, 1)], indexed: Some((1, 1, Indexing::Target)) },
            m("LogicOp", &[f(1, 8, 4)]),
            m("AlphaTest", &[f(1, 12, 4)]),
        ],
    },
    Kind {
        prefix: "nvnChannelMaskState",
        size: 4,
        defaults: 0xFFFF_FFFF,
        methods: &[Method {
            name: "ChannelMask",
            fields: &[f(2, 0, 1), f(3, 1, 1), f(4, 2, 1), f(5, 3, 1)],
            indexed: Some((1, 4, Indexing::Target)),
        }],
    },
    Kind {
        prefix: "nvnDepthStencilState",
        size: 8,
        // depth test off, depth write on, less; stencil off, always, keep
        defaults: (1 << 1) | (2 << 2) | (8 << 7) | (8 << 11) | (0x111 << 15) | (0x111 << 27),
        methods: &[
            m("DepthTestEnable", &[f(1, 0, 1)]),
            m("DepthWriteEnable", &[f(1, 1, 1)]),
            m("DepthFunc", &[f(1, 2, 4)]),
            m("StencilTestEnable", &[f(1, 6, 1)]),
            Method { name: "StencilFunc", fields: &[f(2, 7, 4)], indexed: Some((1, 4, Indexing::Face)) },
            Method {
                name: "StencilOp",
                fields: &[f(2, 15, 4), f(3, 19, 4), f(4, 23, 4)],
                indexed: Some((1, 12, Indexing::Face)),
            },
        ],
    },
    Kind {
        prefix: "nvnBlendState",
        size: 8,
        // target 0, one/zero for color and alpha, add for both
        defaults: (2 << 3) | (1 << 8) | (2 << 13) | (1 << 18) | (1 << 23) | (1 << 26),
        methods: &[
            m("BlendTarget", &[f(1, 0, 3)]),
            m("BlendFunc", &[f(1, 3, 5), f(2, 8, 5), f(3, 13, 5), f(4, 18, 5)]),
            m("BlendEquation", &[f(1, 23, 3), f(2, 26, 3)]),
            m("AdvancedMode", &[f(1, 29, 7)]),
            m("AdvancedOverlap", &[f(1, 36, 2)]),
            m("AdvancedPremultipliedSrc", &[f(1, 38, 1)]),
            m("AdvancedNormalizedDst", &[f(1, 39, 1)]),
        ],
    },
    Kind {
        prefix: "nvnMultisampleState",
        size: 8,
        // multisampling on, alpha-to-coverage dither on
        defaults: 1 | (1 << 7),
        methods: &[
            m("MultisampleEnable", &[f(1, 0, 1)]),
            m("Samples", &[f(1, 1, 5)]),
            m("AlphaToCoverageEnable", &[f(1, 6, 1)]),
            m("AlphaToCoverageDither", &[f(1, 7, 1)]),
            m("RasterSamples", &[f(1, 8, 5)]),
            m("CoverageModulationMode", &[f(1, 13, 2)]),
            m("CoverageTruncationEnable", &[f(1, 15, 1)]),
            m("CoverageToColorEnable", &[f(1, 16, 1)]),
            m("CoverageToColorOutput", &[f(1, 17, 3)]),
            m("SampleLocationsEnable", &[f(1, 20, 1)]),
            m("SampleLocationsGridEnable", &[f(1, 21, 1)]),
        ],
    },
    Kind {
        prefix: "nvnVertexAttribState",
        size: 4,
        defaults: 0,
        methods: &[m("Format", &[f(1, 0, 10), f(2, 10, 14)]), m("StreamIndex", &[f(1, 24, 5)])],
    },
    Kind {
        prefix: "nvnVertexStreamState",
        size: 8,
        defaults: 0,
        methods: &[m("Stride", &[f(1, 0, 32)]), m("Divisor", &[f(1, 32, 32)])],
    },
];

fn read_word(state: &State, addr: u64, size: u64) -> Result<u64, HostResult> {
    let b = read_bytes(state, addr, size.min(8))?;
    let mut w = [0u8; 8];
    w[..b.len()].copy_from_slice(&b);
    Ok(u64::from_le_bytes(w))
}

fn write_word(state: &mut State, addr: u64, size: u64, v: u64) -> Result<(), HostResult> {
    write_bytes(state, addr, &v.to_le_bytes()[..size.min(8) as usize])
}

fn mask(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// The bit shifts a method applies to: one per target, or one per face named
/// by the face mask.
fn shifts(method: &Method, args: &[u64]) -> Vec<u32> {
    match method.indexed {
        None => vec![0],
        Some((arg, stride, Indexing::Target)) => vec![(args[arg] as u32 & 7) * stride],
        Some((arg, stride, Indexing::Face)) => {
            let faces = args[arg] as u32;
            [(1u32, 0u32), (2, 1)].iter().filter(|(bit, _)| faces & bit != 0).map(|(_, i)| i * stride).collect()
        }
    }
}

fn lookup(name: &str) -> Option<(&'static Kind, Option<&'static Method>, bool)> {
    let kind = KINDS.iter().find(|k| name.starts_with(k.prefix))?;
    let rest = &name[kind.prefix.len()..];
    if rest == "SetDefaults" {
        return Some((kind, None, true));
    }
    let (set, field) = if let Some(f) = rest.strip_prefix("Set") { (true, f) } else { (false, rest.strip_prefix("Get")?) };
    kind.methods.iter().find(|m| m.name == field).map(|m| (kind, Some(m), set))
}

fn handle(state: &mut State) -> HostResult {
    let name = called(state);
    let Some((kind, method, set)) = lookup(&name) else {
        return HostResult::Unavailable(format!("{name} is not modelled"));
    };
    let obj = state.x[0];
    let args: Vec<u64> = state.x[0..8].to_vec();
    let Some(method) = method else {
        return match write_word(state, obj, kind.size, kind.defaults) {
            Ok(()) => HostResult::Returned(0),
            Err(e) => e,
        };
    };
    let mut word = match read_word(state, obj, kind.size) {
        Ok(w) => w,
        Err(e) => return e,
    };
    let shifts = shifts(method, &args);
    if set {
        for s in &shifts {
            for fl in method.fields {
                let at = fl.at + s;
                word = (word & !(mask(fl.bits) << at)) | ((args[fl.arg] & mask(fl.bits)) << at);
            }
        }
        return match write_word(state, obj, kind.size, word) {
            Ok(()) => HostResult::Returned(0),
            Err(e) => e,
        };
    }
    // A getter reads one face or target. Single-field getters without an
    // index return the value; the rest write through their out pointers,
    // which follow the index argument when there is one.
    let s = shifts.first().copied().unwrap_or(0);
    let value = |fl: &F| (word >> (fl.at + s)) & mask(fl.bits);
    if method.fields.len() == 1 && method.indexed.is_none() {
        return HostResult::Returned(value(&method.fields[0]));
    }
    if method.fields.len() == 1 {
        return HostResult::Returned(value(&method.fields[0]));
    }
    let first_out = if method.indexed.is_some() { 2 } else { 1 };
    for (i, fl) in method.fields.iter().enumerate() {
        let out = args[first_out + i];
        if out != 0 {
            if let Err(e) = write_bytes(state, out, &(value(fl) as u32).to_le_bytes()) {
                return e;
            }
        }
    }
    HostResult::Returned(0)
}

/// Bytes of a state structure a bind should copy, by the bind procedure.
#[allow(dead_code)]
pub fn bind_size(bind: &str) -> Option<u64> {
    let kind = bind.trim_start_matches("nvnCommandBufferBind");
    KINDS.iter().find(|k| k.prefix.trim_start_matches("nvn") == kind).map(|k| k.size)
}

pub fn register(s: &mut HostServices) {
    for k in KINDS {
        s.register_prefix(&format!("nvn:{}", k.prefix), handle, "state packed into the structure's own bytes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(s: &mut State, name: &str, args: &[u64]) -> HostResult {
        s.host.dyn_base = 0x1000;
        s.host.dyn_stubs = vec![format!("nvn:{name}")];
        s.pc = 0x1000;
        for (i, a) in args.iter().enumerate() {
            s.x[i] = *a;
        }
        handle(s)
    }

    #[test]
    fn setters_pack_and_getters_unpack() {
        let mut s = State { mem: vec![0; 0x2000], ..State::default() };
        assert_eq!(call(&mut s, "nvnDepthStencilStateSetDefaults", &[0x100]), HostResult::Returned(0));
        assert_eq!(call(&mut s, "nvnDepthStencilStateGetDepthFunc", &[0x100]), HostResult::Returned(2));
        call(&mut s, "nvnDepthStencilStateSetDepthFunc", &[0x100, 4]);
        assert_eq!(call(&mut s, "nvnDepthStencilStateGetDepthFunc", &[0x100]), HostResult::Returned(4));
        // Back face only: the front face keeps its default.
        call(&mut s, "nvnDepthStencilStateSetStencilFunc", &[0x100, 2, 3]);
        assert_eq!(call(&mut s, "nvnDepthStencilStateGetStencilFunc", &[0x100, 1]), HostResult::Returned(8));
        assert_eq!(call(&mut s, "nvnDepthStencilStateGetStencilFunc", &[0x100, 2]), HostResult::Returned(3));
        // Out-pointer getter.
        call(&mut s, "nvnBlendStateSetDefaults", &[0x200]);
        call(&mut s, "nvnBlendStateSetBlendFunc", &[0x200, 5, 6, 2, 1]);
        call(&mut s, "nvnBlendStateGetBlendFunc", &[0x200, 0x300, 0x304, 0x308, 0x30c]);
        assert_eq!(&s.mem[0x300..0x310], &[5, 0, 0, 0, 6, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn a_copied_structure_keeps_its_state() {
        let mut s = State { mem: vec![0; 0x2000], ..State::default() };
        call(&mut s, "nvnPolygonStateSetDefaults", &[0x100]);
        call(&mut s, "nvnPolygonStateSetCullFace", &[0x100, 2]);
        let bytes = s.mem[0x100..0x104].to_vec();
        s.mem[0x500..0x504].copy_from_slice(&bytes);
        assert_eq!(call(&mut s, "nvnPolygonStateGetCullFace", &[0x500]), HostResult::Returned(2));
        assert_eq!(call(&mut s, "nvnPolygonStateGetPolygonMode", &[0x500]), HostResult::Returned(2));
    }
}
