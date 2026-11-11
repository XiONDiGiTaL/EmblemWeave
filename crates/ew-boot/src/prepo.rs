//! `nn::prepo` play reports: telemetry the game builds and hands to the system
//! to upload. The host accepts every report and sends nothing anywhere. It
//! counts the entries added to each report so `GetCount` answers truthfully,
//! and `Clear` resets the count.

use crate::hle::{HostResult, HostServices};
use ew_lift::State;
use std::collections::HashMap;

#[derive(Default)]
struct Reports {
    counts: HashMap<u64, u64>,
}

fn reports<R>(state: &State, f: impl FnOnce(&mut Reports) -> R) -> R {
    state.host.ext.with::<Reports, R>(f)
}

/// Constructors, buffers, event ids, and `Save`: accepted, nothing kept.
fn accept(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

fn add(state: &mut State) -> HostResult {
    let object = state.x[0];
    reports(state, |r| *r.counts.entry(object).or_insert(0) += 1);
    HostResult::Returned(0)
}

fn clear(state: &mut State) -> HostResult {
    let object = state.x[0];
    reports(state, |r| r.counts.remove(&object));
    HostResult::Returned(0)
}

fn count(state: &mut State) -> HostResult {
    let object = state.x[0];
    HostResult::Returned(reports(state, |r| r.counts.get(&object).copied().unwrap_or(0)))
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult)] = &[
        ("_ZN2nn5prepo10PlayReportC1Ev", clear),
        ("_ZN2nn5prepo10PlayReportD1Ev", clear),
        ("_ZN2nn5prepo10PlayReport9SetBufferEPvm", clear),
        ("_ZN2nn5prepo10PlayReport10SetEventIdEPKc", accept),
        ("_ZN2nn5prepo10PlayReport5ClearEv", clear),
        ("_ZN2nn5prepo10PlayReport3AddEPKcb", add),
        ("_ZN2nn5prepo10PlayReport3AddEPKcS3_", add),
        ("_ZN2nn5prepo10PlayReport3AddEPKcl", add),
        ("_ZN2nn5prepo10PlayReport3AddEPKcRKNS0_10Any64BitIdE", add),
        ("_ZN2nn5prepo10PlayReport3AddEPKcRKNS0_5ArrayE", add),
        ("_ZNK2nn5prepo10PlayReport8GetCountEv", count),
        ("_ZN2nn5prepo10PlayReport4SaveERKNS_7account3UidE", accept),
        ("_ZN2nn5prepo10PlayReport4SaveEv", accept),
        ("_ZN2nn5prepo6StructC1Ev", clear),
        ("_ZN2nn5prepo6Struct9SetBufferEPvm", clear),
        ("_ZN2nn5prepo6Struct5ClearEv", clear),
        ("_ZN2nn5prepo6Struct3AddEPKcb", add),
        ("_ZN2nn5prepo6Struct3AddEPKcS3_", add),
        ("_ZN2nn5prepo6Struct3AddEPKcl", add),
        ("_ZN2nn5prepo6Struct3AddEPKcRKNS0_5ArrayE", add),
        ("_ZN2nn5prepo6Struct3AddEPKcRKS1_", add),
        ("_ZN2nn5prepo5ArrayC1Ev", clear),
        ("_ZN2nn5prepo5Array9SetBufferEPvm", clear),
        ("_ZN2nn5prepo5Array5ClearEv", clear),
        ("_ZN2nn5prepo5Array3AddEPKc", add),
        ("_ZN2nn5prepo5Array3AddEl", add),
        ("_ZN2nn5prepo5Array3AddERKNS0_6StructE", add),
    ];
    for (name, handler) in table {
        s.register(name, *handler, "play reports are accepted and never sent");
    }
}
