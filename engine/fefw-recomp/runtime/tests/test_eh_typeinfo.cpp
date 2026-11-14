// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for nxrt::eh::can_catch and ti_kind / ti_name / ti_equal (src/eh/typeinfo.cpp): the Itanium typeinfo matcher.
// The guest's std::type_info objects are built as ordinary host structs with the LP64 layout; their addresses are passed as
// uint64_t. A typeinfo of kind K has vptr = (address of K's vtable symbol) + 16; the symbols here are local arrays.
#include "test_framework.h"
#include "nxrt/eh.h"

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <string>

NXTEST_GROUP(eh_typeinfo)

namespace {

// Fake vtable symbols: only their addresses matter.
void* vt_fundamental[8];
void* vt_array[8];
void* vt_function[8];
void* vt_enum[8];
void* vt_klass[8];
void* vt_si[8];
void* vt_vmi[8];
void* vt_ptr[8];
void* vt_ptm[8];

const void* vptr_of(void* const* vt) { return static_cast<const void*>(vt + 2); }
uint64_t ad(const void* p) { return static_cast<uint64_t>(reinterpret_cast<uintptr_t>(p)); }

void use_vtables() {
    nxrt::eh::TypeinfoVtables v;
    v.fundamental = ad(vt_fundamental);
    v.array = ad(vt_array);
    v.function = ad(vt_function);
    v.enumeration = ad(vt_enum);
    v.klass = ad(vt_klass);
    v.si_class = ad(vt_si);
    v.vmi_class = ad(vt_vmi);
    v.pointer = ad(vt_ptr);
    v.pointer_to_member = ad(vt_ptm);
    nxrt::eh::set_typeinfo_vtables(v);
}

// LP64 layouts of the guest's typeinfo objects (vptr at 0, name at 8 in every one).
struct TiPlain { const void* vptr; const char* name; };                                   // fundamental, enum, array, function, class
struct TiSi { const void* vptr; const char* name; const void* base; };                     // __si_class_type_info
struct TiVmiBase { const void* base; int64_t offset_flags; };
template <size_t N>
struct TiVmi { const void* vptr; const char* name; uint32_t flags; uint32_t count; TiVmiBase bases[N]; };
struct TiPtr { const void* vptr; const char* name; uint32_t flags; uint32_t pad; const void* pointee; };  // pointer; also the ptm header
// Object of class D : L, R where L and R each have a virtual base V (layout of the object, not of a typeinfo).
struct DiamondObj { const void* L_vptr; uint64_t l; const void* R_vptr; uint64_t r; uint64_t d; uint64_t v; };

static_assert(offsetof(TiSi, base) == 16);
static_assert(offsetof(TiVmi<1>, bases) == 24);
static_assert(offsetof(TiPtr, pointee) == 24);
static_assert(offsetof(DiamondObj, R_vptr) == 16 && offsetof(DiamondObj, d) == 32 && offsetof(DiamondObj, v) == 40);

// __base_class_type_info::__offset_flags: offset << 8 | public (2) | virtual (1). Written as offset * 256 so negative offsets are clean.
constexpr int64_t base_flags(int64_t offset, bool is_public, bool is_virtual) {
    return offset * 256 + (is_public ? 2 : 0) + (is_virtual ? 1 : 0);
}

}  // namespace

NXTEST(exact_types) {
    use_vtables();
    TiPlain i1{vptr_of(vt_fundamental), "i"};
    TiPlain i2{vptr_of(vt_fundamental), "i"};
    TiPlain l1{vptr_of(vt_fundamental), "l"};
    alignas(16) unsigned char buf[64] = {};
    uint64_t adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&i1), ad(&i1), &adj));
    CHECK(nxrt::eh::can_catch(ad(&i1), ad(&i2), &adj));           // same name, different object
    CHECK(!nxrt::eh::can_catch(ad(&i1), ad(&l1), &adj));          // int vs long
    CHECK(nxrt::eh::can_catch(0, ad(&l1), &adj));                 // catch (...) takes anything
    CHECK(nxrt::eh::can_catch(0, 0, &adj));                       // catch (...) takes a foreign exception
    CHECK(!nxrt::eh::can_catch(ad(&i1), 0, &adj));                // a typed catch does not take a foreign exception

    // Names starting with '*' have internal linkage: equal only to the same object. Two distinct buffers with the same text.
    static char anon_a[] = "*anon";
    static char anon_b[] = "*anon";
    TiPlain s1{vptr_of(vt_fundamental), anon_a};
    TiPlain s2{vptr_of(vt_fundamental), anon_b};
    CHECK(!nxrt::eh::can_catch(ad(&s1), ad(&s2), &adj));
    CHECK(nxrt::eh::can_catch(ad(&s1), ad(&s1), &adj));
}

NXTEST(single_inheritance) {
    use_vtables();
    TiPlain A{vptr_of(vt_klass), "1A"};
    TiSi B{vptr_of(vt_si), "1B", &A};
    TiSi C{vptr_of(vt_si), "1C", &B};
    TiPlain X{vptr_of(vt_klass), "1X"};
    alignas(16) unsigned char buf[64] = {};

    uint64_t adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&A), ad(&C), &adj));            // grandparent catch
    CHECK_EQ(adj, ad(buf));                                       // single inheritance: offset 0, adjusted unchanged

    adj = ad(buf);
    CHECK(!nxrt::eh::can_catch(ad(&C), ad(&A), &adj));            // a base does not catch its derived type
    adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&B), ad(&C), &adj));             // parent catch
    adj = ad(buf);
    CHECK(!nxrt::eh::can_catch(ad(&X), ad(&C), &adj));            // unrelated class
}

NXTEST(multiple_inheritance_offset) {
    use_vtables();
    TiPlain P{vptr_of(vt_klass), "1P"};
    TiPlain Q{vptr_of(vt_klass), "1Q"};
    TiVmi<2> M{vptr_of(vt_vmi), "1M", 0, 2, {{&P, base_flags(0, true, false)}, {&Q, base_flags(16, true, false)}}};
    alignas(16) unsigned char buf[64] = {};

    uint64_t adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&Q), ad(&M), &adj));
    CHECK_EQ(adj, ad(buf) + 16);                                  // second base sits at offset 16

    adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&P), ad(&M), &adj));
    CHECK_EQ(adj, ad(buf));                                       // first base at offset 0

    adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&M), ad(&M), &adj));
    CHECK_EQ(adj, ad(buf));                                       // exact type: adjusted unchanged
}

NXTEST(private_and_protected_base) {
    use_vtables();
    TiPlain Pub{vptr_of(vt_klass), "1Pub"};
    TiPlain Priv{vptr_of(vt_klass), "1Priv"};
    TiVmi<1> OnlyPrivate{vptr_of(vt_vmi), "1OnlyPrivate", 0, 1, {{&Priv, base_flags(0, false, false)}}};
    TiVmi<2> Mixed{vptr_of(vt_vmi), "1Mixed", 0, 2, {{&Pub, base_flags(0, true, false)}, {&Priv, base_flags(16, false, false)}}};
    alignas(16) unsigned char buf[64] = {};

    uint64_t adj = ad(buf);
    CHECK(!nxrt::eh::can_catch(ad(&Priv), ad(&OnlyPrivate), &adj));  // a non-public base is not catchable
    adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&Pub), ad(&Mixed), &adj));          // the public base still is
    CHECK_EQ(adj, ad(buf));
    adj = ad(buf);
    CHECK(!nxrt::eh::can_catch(ad(&Priv), ad(&Mixed), &adj));        // the private one in the same class is not
}

NXTEST(ambiguous_base) {
    use_vtables();
    TiPlain A{vptr_of(vt_klass), "1A"};
    TiSi B1{vptr_of(vt_si), "1B1", &A};
    TiSi B2{vptr_of(vt_si), "1B2", &A};
    TiVmi<2> D{vptr_of(vt_vmi), "1D", 0, 2, {{&B1, base_flags(0, true, false)}, {&B2, base_flags(16, true, false)}}};
    alignas(16) unsigned char buf[64] = {};

    uint64_t adj = ad(buf);
    CHECK(!nxrt::eh::can_catch(ad(&A), ad(&D), &adj));               // A appears twice (two subobjects): ambiguous
    adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&B1), ad(&D), &adj));               // B1 appears once
    CHECK_EQ(adj, ad(buf));
}

NXTEST(virtual_diamond) {
    use_vtables();
    TiPlain V{vptr_of(vt_klass), "1V"};
    // Instance vtables: the vbase offset of a subobject sits at vptr - 24 (index 0 when vptr = &vtbl[3]).
    int64_t vtbl_L[4] = {40, 0, 0, 0};    // from the L subobject at object+0 to V at object+40
    int64_t vtbl_R[4] = {24, 0, 0, 0};    // from the R subobject at object+16 to V at object+40
    // Typeinfos: the vbase offset field is -24 bytes from the instance vptr, marked virtual and public.
    TiVmi<1> L{vptr_of(vt_vmi), "1L", 0, 1, {{&V, base_flags(-24, true, true)}}};
    TiVmi<1> R{vptr_of(vt_vmi), "1R", 0, 1, {{&V, base_flags(-24, true, true)}}};
    TiVmi<2> D{vptr_of(vt_vmi), "1D", 0, 2, {{&L, base_flags(0, true, false)}, {&R, base_flags(16, true, false)}}};

    DiamondObj obj{};
    obj.L_vptr = &vtbl_L[3];
    obj.R_vptr = &vtbl_R[3];

    uint64_t adj = ad(&obj);
    CHECK(nxrt::eh::can_catch(ad(&V), ad(&D), &adj));               // V is one shared subobject: not ambiguous
    CHECK_EQ(adj, ad(&obj.v));

    adj = ad(&obj);
    CHECK(nxrt::eh::can_catch(ad(&L), ad(&D), &adj));
    CHECK_EQ(adj, ad(&obj));

    adj = ad(&obj);
    CHECK(nxrt::eh::can_catch(ad(&R), ad(&D), &adj));
    CHECK_EQ(adj, ad(&obj.R_vptr));                                  // R at object+16

    adj = ad(&obj);
    CHECK(nxrt::eh::can_catch(ad(&V), ad(&L), &adj));               // the virtual base seen from L alone
    CHECK_EQ(adj, ad(&obj.v));
}

NXTEST(pointer_exact_and_qualification) {
    use_vtables();
    TiPlain i{vptr_of(vt_fundamental), "i"};
    TiPtr pint{vptr_of(vt_ptr), "Pi", 0, 0, &i};            // int*
    TiPtr pconst{vptr_of(vt_ptr), "PKi", 1, 0, &i};         // const int*   (pointee const)
    TiPtr pvol{vptr_of(vt_ptr), "PVi", 2, 0, &i};           // volatile int*
    int value = 42;
    uint64_t cell = ad(&value);                              // the thrown object: a cell holding the pointer value
    uint64_t adj = ad(&cell);

    CHECK(nxrt::eh::can_catch(ad(&pint), ad(&pint), &adj));
    CHECK_EQ(adj, ad(&value));                               // handler gets the pointer value

    adj = ad(&cell);
    CHECK(nxrt::eh::can_catch(ad(&pconst), ad(&pint), &adj));   // adding const is allowed
    CHECK_EQ(adj, ad(&value));

    adj = ad(&cell);
    CHECK(!nxrt::eh::can_catch(ad(&pint), ad(&pconst), &adj));  // removing const is not
    adj = ad(&cell);
    CHECK(!nxrt::eh::can_catch(ad(&pint), ad(&pvol), &adj));    // removing volatile is not
}

NXTEST(pointer_to_void) {
    use_vtables();
    TiPlain i{vptr_of(vt_fundamental), "i"};
    TiPlain v{vptr_of(vt_fundamental), "v"};
    TiPlain fn{vptr_of(vt_function), "FvvE"};
    TiPtr pint{vptr_of(vt_ptr), "Pi", 0, 0, &i};
    TiPtr pvoid{vptr_of(vt_ptr), "Pv", 0, 0, &v};
    TiPtr pfn{vptr_of(vt_ptr), "PFvvE", 0, 0, &fn};
    int value = 7;
    uint64_t cell = ad(&value);
    uint64_t adj = ad(&cell);

    CHECK(nxrt::eh::can_catch(ad(&pvoid), ad(&pint), &adj));    // any object pointer converts to void*
    CHECK_EQ(adj, ad(&value));

    adj = ad(&cell);
    CHECK(!nxrt::eh::can_catch(ad(&pvoid), ad(&pfn), &adj));    // a function pointer does not convert to void*

    adj = ad(&cell);
    CHECK(!nxrt::eh::can_catch(ad(&pint), ad(&pvoid), &adj));   // void* does not convert back to int*
}

NXTEST(pointer_derived_to_base) {
    use_vtables();
    TiPlain P{vptr_of(vt_klass), "1P"};
    TiPlain Q{vptr_of(vt_klass), "1Q"};
    TiVmi<2> M{vptr_of(vt_vmi), "1M", 0, 2, {{&P, base_flags(0, true, false)}, {&Q, base_flags(16, true, false)}}};
    TiPtr pM{vptr_of(vt_ptr), "P1M", 0, 0, &M};
    TiPtr pQ{vptr_of(vt_ptr), "P1Q", 0, 0, &Q};
    TiPtr pP{vptr_of(vt_ptr), "P1P", 0, 0, &P};
    alignas(16) unsigned char buf[64] = {};
    uint64_t cell = ad(buf);                                 // the thrown M* value is buf
    uint64_t adj = ad(&cell);

    CHECK(nxrt::eh::can_catch(ad(&pQ), ad(&pM), &adj));      // M* -> Q* adds the base offset
    CHECK_EQ(adj, ad(buf) + 16);

    adj = ad(&cell);
    CHECK(nxrt::eh::can_catch(ad(&pP), ad(&pM), &adj));      // M* -> P* at offset 0
    CHECK_EQ(adj, ad(buf));

    adj = ad(&cell);
    CHECK(nxrt::eh::can_catch(ad(&pM), ad(&pM), &adj));      // exact
    CHECK_EQ(adj, ad(buf));

    uint64_t cellQ = ad(buf) + 16;                           // thrown Q* pointing at the Q subobject
    adj = ad(&cellQ);
    CHECK(!nxrt::eh::can_catch(ad(&pM), ad(&pQ), &adj));     // a base pointer does not convert to the derived pointer
}

NXTEST(pointer_null) {
    use_vtables();
    TiPlain P{vptr_of(vt_klass), "1P"};
    TiPlain Q{vptr_of(vt_klass), "1Q"};
    TiVmi<2> M{vptr_of(vt_vmi), "1M", 0, 2, {{&P, base_flags(0, true, false)}, {&Q, base_flags(16, true, false)}}};
    TiPtr pM{vptr_of(vt_ptr), "P1M", 0, 0, &M};
    TiPtr pQ{vptr_of(vt_ptr), "P1Q", 0, 0, &Q};
    TiPlain dn{vptr_of(vt_fundamental), "Dn"};
    TiPlain i{vptr_of(vt_fundamental), "i"};

    uint64_t cell = 0;                                       // null M*
    uint64_t adj = ad(&cell);
    CHECK(nxrt::eh::can_catch(ad(&pQ), ad(&pM), &adj));      // null converts to a base pointer
    CHECK_EQ(adj, static_cast<uint64_t>(0));                 // and stays null: no offset added

    adj = ad(&cell);
    CHECK(nxrt::eh::can_catch(ad(&pM), ad(&pM), &adj));      // exact
    CHECK_EQ(adj, static_cast<uint64_t>(0));

    uint64_t dn_obj = 0;                                     // nullptr_t object
    adj = ad(&dn_obj);
    CHECK(nxrt::eh::can_catch(ad(&pQ), ad(&dn), &adj));      // nullptr_t catches any pointer type
    CHECK_EQ(adj, static_cast<uint64_t>(0));

    adj = ad(&dn_obj);
    CHECK(!nxrt::eh::can_catch(ad(&i), ad(&dn), &adj));      // but not int
}

NXTEST(pointer_to_pointer) {
    use_vtables();
    TiPlain i{vptr_of(vt_fundamental), "i"};
    TiPtr pPlain{vptr_of(vt_ptr), "Pi", 0, 0, &i};          // int*
    TiPtr pp{vptr_of(vt_ptr), "PPi", 0, 0, &pPlain};        // int**
    TiPtr pConst{vptr_of(vt_ptr), "PKi", 1, 0, &i};         // const int*
    TiPtr ppConst{vptr_of(vt_ptr), "PKPKi", 1, 0, &pConst}; // const int* const*  (pointee: const pointer to const int)
    int value = 3;
    int* ip = &value;
    int** ipp = &ip;                                         // a thrown int** value
    uint64_t cell = ad(ipp);
    uint64_t adj = ad(&cell);

    CHECK(nxrt::eh::can_catch(ad(&pp), ad(&pp), &adj));     // exact
    CHECK_EQ(adj, ad(ipp));

    adj = ad(&cell);
    CHECK(nxrt::eh::can_catch(ad(&ppConst), ad(&pp), &adj));  // int** -> const int* const*: const added at every level
    CHECK_EQ(adj, ad(ipp));

    uint64_t cell2 = ad(ipp);
    adj = ad(&cell2);
    CHECK(!nxrt::eh::can_catch(ad(&pp), ad(&ppConst), &adj)); // the reverse removes const: not allowed
}

NXTEST(enum_and_fundamental_catch_do_not_convert) {
    use_vtables();
    TiPlain E{vptr_of(vt_enum), "3Foo"};
    TiPlain E2{vptr_of(vt_enum), "3Foo"};
    TiPlain i{vptr_of(vt_fundamental), "i"};
    TiPlain arr{vptr_of(vt_array), "A4_i"};
    TiPlain fn{vptr_of(vt_function), "FvvE"};
    uint64_t obj = 0;
    uint64_t adj = ad(&obj);

    CHECK(!nxrt::eh::can_catch(ad(&E), ad(&i), &adj));       // enum does not catch int
    CHECK(!nxrt::eh::can_catch(ad(&i), ad(&E), &adj));       // int does not catch enum
    CHECK(nxrt::eh::can_catch(ad(&E), ad(&E2), &adj));       // same enum, different object
    CHECK(!nxrt::eh::can_catch(ad(&arr), ad(&arr), &adj));   // array types are never caught
    CHECK(!nxrt::eh::can_catch(ad(&fn), ad(&fn), &adj));     // function types are never caught
}

NXTEST(unknown_kind_falls_back_to_identity) {
    // All vtable symbols zeroed: no typeinfo is recognised, so every class is Unknown.
    nxrt::eh::set_typeinfo_vtables(nxrt::eh::TypeinfoVtables{});
    TiPlain A{vptr_of(vt_klass), "1A"};
    TiPlain A2{vptr_of(vt_klass), "1A"};
    TiSi B{vptr_of(vt_si), "1B", &A};
    alignas(16) unsigned char buf[64] = {};
    uint64_t adj = ad(buf);
    CHECK(nxrt::eh::can_catch(ad(&A), ad(&A2), &adj));       // exact (same name) still matches
    CHECK(!nxrt::eh::can_catch(ad(&A), ad(&B), &adj));       // derived-to-base cannot be read without the kinds
    CHECK_EQ(adj, ad(buf));
}

NXTEST(ti_kind_and_names) {
    use_vtables();
    TiPlain fund{vptr_of(vt_fundamental), "i"};
    TiPlain arr{vptr_of(vt_array), "A4_i"};
    TiPlain fn{vptr_of(vt_function), "FvvE"};
    TiPlain en{vptr_of(vt_enum), "3Foo"};
    TiPlain cls{vptr_of(vt_klass), "1A"};
    TiSi si{vptr_of(vt_si), "1B", &cls};
    TiVmi<1> vmi{vptr_of(vt_vmi), "1M", 0, 1, {{&cls, base_flags(0, true, false)}}};
    TiPtr ptr{vptr_of(vt_ptr), "Pi", 0, 0, &fund};
    TiPtr ptm{vptr_of(vt_ptm), "M1AFvvE", 0, 0, &fn};

    CHECK(nxrt::eh::ti_kind(ad(&fund)) == nxrt::eh::TiKind::Fundamental);
    CHECK(nxrt::eh::ti_kind(ad(&arr)) == nxrt::eh::TiKind::Array);
    CHECK(nxrt::eh::ti_kind(ad(&fn)) == nxrt::eh::TiKind::Function);
    CHECK(nxrt::eh::ti_kind(ad(&en)) == nxrt::eh::TiKind::Enum);
    CHECK(nxrt::eh::ti_kind(ad(&cls)) == nxrt::eh::TiKind::Class);
    CHECK(nxrt::eh::ti_kind(ad(&si)) == nxrt::eh::TiKind::SiClass);
    CHECK(nxrt::eh::ti_kind(ad(&vmi)) == nxrt::eh::TiKind::VmiClass);
    CHECK(nxrt::eh::ti_kind(ad(&ptr)) == nxrt::eh::TiKind::Pointer);
    CHECK(nxrt::eh::ti_kind(ad(&ptm)) == nxrt::eh::TiKind::PointerToMember);
    CHECK(nxrt::eh::ti_kind(0) == nxrt::eh::TiKind::Unknown);

    CHECK(std::strcmp(nxrt::eh::ti_name(ad(&si)), "1B") == 0);
    CHECK(std::strcmp(nxrt::eh::ti_name(ad(&ptm)), "M1AFvvE") == 0);
    CHECK(std::strcmp(nxrt::eh::ti_name(0), "") == 0);
}

NXTEST(search_budget_and_cycles) {
    use_vtables();
    TiPlain T{vptr_of(vt_klass), "1T"};
    alignas(16) unsigned char buf[64] = {};
    uint64_t adj = ad(buf);

    // Corrupt header: 100000 bases declared, only one present. Must be rejected without reading past it.
    TiVmi<1> big{vptr_of(vt_vmi), "1Big", 0, 100000, {{&T, base_flags(0, true, false)}}};
    CHECK(!nxrt::eh::can_catch(ad(&T), ad(&big), &adj));

    // A vmi class whose only base is itself: the search budget must end it.
    TiVmi<1> cyc{vptr_of(vt_vmi), "1Cyc", 0, 1, {{nullptr, base_flags(0, true, false)}}};
    cyc.bases[0].base = &cyc;
    adj = ad(buf);
    CHECK(!nxrt::eh::can_catch(ad(&T), ad(&cyc), &adj));

    // A si class whose base is itself: same guard, the single-inheritance path.
    TiSi si{vptr_of(vt_si), "1S", nullptr};
    si.base = &si;
    adj = ad(buf);
    CHECK(!nxrt::eh::can_catch(ad(&T), ad(&si), &adj));
}

NXTEST(non_unique_name_bit_is_masked) {
    // ARM64 libc++ marks a non-unique typeinfo name with bit 63 of the name pointer; the name is still the string behind the low bits.
    use_vtables();
    static const char name[] = "7Widget";
    static const char name_copy[] = "7Widget";
    TiPlain a = {vptr_of(vt_klass), nullptr}, b = {vptr_of(vt_klass), nullptr};
    uint64_t pa = reinterpret_cast<uint64_t>(name) | (1ull << 63), pb = reinterpret_cast<uint64_t>(name_copy);
    std::memcpy(&a.name, &pa, 8);
    std::memcpy(&b.name, &pb, 8);
    CHECK(std::string(nxrt::eh::ti_name(ad(&a))) == "7Widget");
    CHECK(nxrt::eh::ti_equal(ad(&a), ad(&b)));          // two objects, same string (one with the bit set)
    uint64_t adj = 0x1000;
    CHECK(nxrt::eh::can_catch(ad(&a), ad(&b), &adj));
    CHECK_EQ(adj, 0x1000u);
}
