//! `std::pmr`, the polymorphic memory resources of libc++.
//!
//! libc++ keeps one process-wide default resource, initially
//! `std::pmr::new_delete_resource()`. That resource is an object with a vtable,
//! and its allocation methods call the global `operator new` and
//! `operator delete`. This title replaces those operators with its own (it
//! exports `_Znwm`, `_ZnwmSt11align_val_t` and the matching deletes), so the
//! exact behaviour is a vtable whose entries forward into the game's own code.
//!
//! The host therefore builds a real object in guest memory: a vtable pointer,
//! and an Itanium-layout vtable whose entries are host stubs. Each stub moves
//! the arguments into place and tail-calls the game's operator, which is what
//! libc++'s `__libcpp_allocate` and `__libcpp_deallocate` do: alignments above
//! the default new alignment (16) use the `align_val_t` overloads.

use crate::hle::{HostResult, HostServices};
use ew_lift::State;

/// Host functions with no import slot, reachable only through the objects
/// built here. The dispatcher gives each one its own stub address.
pub const SYNTHETIC: [&str; 5] = [
    "<host> new_delete_resource::~new_delete_resource",
    "<host> new_delete_resource::~new_delete_resource (deleting)",
    "<host> new_delete_resource::do_allocate",
    "<host> new_delete_resource::do_deallocate",
    "<host> new_delete_resource::do_is_equal",
];

/// The default alignment `operator new` guarantees on AArch64.
const NEW_ALIGN: u64 = 16;

/// Lay out the `new_delete_resource` object and its vtable, given the stub
/// address of each entry of [`SYNTHETIC`]. Records the addresses the handlers
/// need in `state.host.named`.
pub fn install(state: &mut State, stubs: &[u64], exports: &[(&str, Option<u64>)]) -> Result<(), String> {
    // offset-to-top, typeinfo, then the five virtual functions; the object is
    // one vtable pointer. typeinfo is null: nothing in this host emits a
    // `dynamic_cast` against it, and a guest that tried would fault visibly.
    let base = state.host.allocate(8 * 8, 16)?;
    let vtable = base + 16;
    let object = base + 16 + 8 * 5;
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    for s in stubs {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    bytes.extend_from_slice(&vtable.to_le_bytes());
    let at = base as usize;
    if at + bytes.len() > state.mem.len() {
        return Err("no room for the pmr resource object".into());
    }
    state.mem[at..at + bytes.len()].copy_from_slice(&bytes);
    state.host.named.insert("pmr.new_delete".into(), object);
    state.host.named.insert("pmr.default".into(), object);
    for (name, addr) in exports {
        if let Some(a) = addr {
            state.host.named.insert(format!("export.{name}"), *a);
        }
    }
    Ok(())
}

fn export(state: &State, name: &str) -> Result<u64, HostResult> {
    state.host.named.get(&format!("export.{name}")).copied().ok_or_else(|| {
        HostResult::Unavailable(format!(
            "new_delete_resource needs `{name}`, which this module does not export"
        ))
    })
}

fn dtor(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// The deleting destructor of a static object is never legitimately called.
fn deleting_dtor(state: &mut State) -> HostResult {
    HostResult::Unavailable(format!(
        "the guest deleted std::pmr::new_delete_resource (object 0x{:x}), which libc++ never does",
        state.x[0]
    ))
}

/// `do_allocate(this, bytes, alignment)`.
fn do_allocate(state: &mut State) -> HostResult {
    let (bytes, align) = (state.x[1], state.x[2]);
    let name = if align > NEW_ALIGN { "_ZnwmSt11align_val_t" } else { "_Znwm" };
    match export(state, name) {
        Ok(target) => {
            state.x[0] = bytes;
            state.x[1] = align;
            HostResult::Jump(target)
        }
        Err(e) => e,
    }
}

/// `do_deallocate(this, p, bytes, alignment)`.
fn do_deallocate(state: &mut State) -> HostResult {
    let (p, align) = (state.x[1], state.x[3]);
    let name = if align > NEW_ALIGN { "_ZdlPvSt11align_val_t" } else { "_ZdlPv" };
    match export(state, name) {
        Ok(target) => {
            state.x[0] = p;
            state.x[1] = align;
            HostResult::Jump(target)
        }
        Err(e) => e,
    }
}

/// `do_is_equal(this, other)`: new_delete_resource is equal only to itself.
fn do_is_equal(state: &mut State) -> HostResult {
    HostResult::Returned((state.x[0] == state.x[1]) as u64)
}

/// `std::pmr::set_default_resource(memory_resource*)`: a null argument restores
/// `new_delete_resource()`; returns the previous default.
fn set_default(state: &mut State) -> HostResult {
    let Some(&new_delete) = state.host.named.get("pmr.new_delete") else {
        return HostResult::Unavailable("the pmr resources were not installed".into());
    };
    let next = if state.x[0] == 0 { new_delete } else { state.x[0] };
    let prev = state.host.named.insert("pmr.default".into(), next).unwrap_or(new_delete);
    HostResult::Returned(prev)
}

fn get_default(state: &mut State) -> HostResult {
    match state.host.named.get("pmr.default") {
        Some(&r) => HostResult::Returned(r),
        None => HostResult::Unavailable("the pmr resources were not installed".into()),
    }
}

/// `std::pmr::memory_resource::~memory_resource()` (base subobject): trivial.
fn base_dtor(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

pub fn register(services: &mut HostServices) {
    let handlers: [fn(&mut State) -> HostResult; 5] =
        [dtor, deleting_dtor, do_allocate, do_deallocate, do_is_equal];
    for (name, handler) in SYNTHETIC.iter().zip(handlers) {
        services.register(name, handler, "libc++ new_delete_resource, forwarding to the game's own operator new/delete");
    }
    services.register(
        "_ZNSt3__13pmr20set_default_resourceEPNS0_15memory_resourceE",
        set_default as _,
        "stores the new default, null restores new_delete_resource(), returns the previous one",
    );
    services.register(
        "_ZNSt3__13pmr20get_default_resourceEv",
        get_default as _,
        "returns the current default, initially new_delete_resource()",
    );
    services.register(
        "_ZNSt3__13pmr15memory_resourceD2Ev",
        base_dtor as _,
        "the base destructor of memory_resource is trivial",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> State {
        let mut s = State::default();
        s.host.alloc_next = 0x10000;
        s.host.alloc_end = 0x20000;
        install(
            &mut s,
            &[0xA0, 0xB0, 0xC0, 0xD0, 0xE0],
            &[("_Znwm", Some(0x1000)), ("_ZnwmSt11align_val_t", Some(0x2000)), ("_ZdlPv", Some(0x3000))],
        )
        .unwrap();
        s
    }

    #[test]
    fn default_resource_round_trips() {
        let mut s = setup();
        let nd = s.host.named["pmr.new_delete"];
        s.x[0] = 0x5000;
        assert_eq!(set_default(&mut s), HostResult::Returned(nd));
        assert_eq!(get_default(&mut s), HostResult::Returned(0x5000));
        s.x[0] = 0;
        assert_eq!(set_default(&mut s), HostResult::Returned(0x5000));
        assert_eq!(get_default(&mut s), HostResult::Returned(nd));
    }

    #[test]
    fn the_object_points_at_its_vtable() {
        let s = setup();
        let obj = s.host.named["pmr.new_delete"] as usize;
        let vt = u64::from_le_bytes(s.mem[obj..obj + 8].try_into().unwrap()) as usize;
        let alloc = u64::from_le_bytes(s.mem[vt + 16..vt + 24].try_into().unwrap());
        assert_eq!(alloc, 0xC0, "slot 2 is do_allocate");
    }

    #[test]
    fn allocation_forwards_to_the_games_operator_new() {
        let mut s = setup();
        s.x = [0; 31];
        s.x[1] = 64;
        s.x[2] = 8;
        assert_eq!(do_allocate(&mut s), HostResult::Jump(0x1000));
        assert_eq!(s.x[0], 64);
        s.x[1] = 64;
        s.x[2] = 64;
        assert_eq!(do_allocate(&mut s), HostResult::Jump(0x2000));
        assert_eq!((s.x[0], s.x[1]), (64, 64));
    }
}
