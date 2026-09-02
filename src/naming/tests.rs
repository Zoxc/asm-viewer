//! The names here are real ones, taken out of the app's own debug binary except where a
//! comment says otherwise: 142 804 text symbols of Rust, C++ and C, whose demangled names
//! average 151 characters and come out of [`short_name`] averaging 21.

use super::*;

#[test]
fn a_path_comes_down_to_its_last_two_segments() {
    assert_eq!(
        short_name("winit::platform_impl::linux::x11::ime"),
        "x11::ime"
    );
    // Nothing to shorten: a C symbol, and a Rust one already at two segments.
    assert_eq!(
        short_name("ts_lexer__invalidate_column_data"),
        "ts_lexer__invalidate_column_data"
    );
    assert_eq!(short_name("Foo::bar"), "Foo::bar");
}

#[test]
fn generic_arguments_go_however_deep_they_nest() {
    assert_eq!(
        short_name("<freya_core::lifecycle::state::State<viewer::ui::locations::Located>>::create"),
        "State::create"
    );
    assert_eq!(
        short_name(
            "core::ptr::drop_glue::\
             <core::array::iter::IntoIter<[core::option::Option<&str>; 1], 1>>"
        ),
        "ptr::drop_glue"
    );
}

/// The `<..>` opening a path is the type an `impl` is on; the one *after* a segment is a
/// turbofish, and its contents are an argument rather than a name. `drop_glue` above is
/// the second and would read `drop_glue::IntoIter` if the two were confused.
#[test]
fn an_impl_qualifier_is_named_after_the_type_it_is_on() {
    assert_eq!(
        short_name(
            "<alloc::vec::Vec<analysis::Symbol> as core::iter::traits::collect::IntoIterator>\
             ::into_iter"
        ),
        "Vec::into_iter"
    );
    assert_eq!(
        short_name(
            "<&mut rustix::net::send_recv::msg::AncillaryIter<std::os::fd::owned::OwnedFd> \
             as core::iter::traits::iterator::IteratorRefSpec>::spec_fold"
        ),
        "AncillaryIter::spec_fold"
    );
}

/// A tuple, a slice or an array has no name of its own, and `::default` alone would not
/// say what the tab is showing.
#[test]
fn a_type_with_no_name_falls_back_to_the_trait() {
    assert_eq!(
        short_name(
            "<(mundy::freedesktop::State, mundy::AvailablePreferences) \
             as core::default::Default>::default"
        ),
        "Default::default"
    );
    assert_eq!(
        short_name("<[T] as core::clone::Clone>::clone"),
        "Clone::clone"
    );
}

/// The innermost one: it is the function the symbol *is*. The ones around it are where it
/// was written, which the two segments before them already say.
#[test]
fn the_closure_a_symbol_is_survives_and_the_ones_around_it_do_not() {
    assert_eq!(
        short_name(
            "<viewer::ui::pad_view::ScratchpadTab as freya_core::element::Component>::render\
             ::{closure#5}::{closure#0}"
        ),
        "ScratchpadTab::render::{closure#0}"
    );
    // The legacy mangling's spelling of the same thing.
    assert_eq!(
        short_name("core::iter::adapters::map::Map<I,F>::next::{{closure}}"),
        "Map::next::{{closure}}"
    );
    assert_eq!(
        short_name("<F as core::ops::function::FnOnce<A>>::call_once::{{vtable.shim}}"),
        "F::call_once::{{vtable.shim}}"
    );
}

/// An item written inside a closure is that item and not the closure, so a marker with a
/// name after it is not the marker the symbol ends in.
#[test]
fn an_item_inside_a_closure_is_named_after_the_item() {
    assert_eq!(short_name("viewer::foo::{closure#0}::inner"), "foo::inner");
}

/// `{:#}` on `rustc_demangle` drops it, which is what `analysis` asks for; this is for a
/// name that reaches a tab by some other route. Both names are written by hand -- the
/// binary this was measured on is mangled `v0` throughout.
#[test]
fn the_legacy_hash_suffix_goes_but_a_function_called_h_something_stays() {
    assert_eq!(
        short_name("std::io::Read::read_to_end::h3a2b1c0d9e8f7a6b"),
        "Read::read_to_end"
    );
    // Fifteen digits, so not a hash. Nor is a lone segment that looks like one.
    assert_eq!(
        short_name("foo::bar::h3a2b1c0d9e8f7a6"),
        "bar::h3a2b1c0d9e8f7a6"
    );
    assert_eq!(short_name("h3a2b1c0d9e8f7a6b"), "h3a2b1c0d9e8f7a6b");
}

#[test]
fn a_cpp_argument_list_goes_and_so_do_the_qualifiers_after_it() {
    assert_eq!(
        short_name(
            "GrSurfaceProxy::Copy(GrRecordingContext*, sk_sp<GrSurfaceProxy>, GrSurfaceOrigin)"
        ),
        "GrSurfaceProxy::Copy"
    );
    assert_eq!(
        short_name("SkImageFilter_Base::getChildOutput(int, skif::Context const&) const"),
        "SkImageFilter_Base::getChildOutput"
    );
    assert_eq!(short_name("SkCodec::~SkCodec()"), "SkCodec::~SkCodec");
}

/// The `<` of an `operator<<` opens no group and the `>` of an `operator>>=` closes none.
/// Read as brackets they swallow the rest of the name.
#[test]
fn an_operator_is_a_name_and_not_a_bracket() {
    assert_eq!(
        short_name("swift::Demangle::DemanglerPrinter::operator<<(long long) &"),
        "DemanglerPrinter::operator<<"
    );
    assert_eq!(
        short_name(
            "(anonymous namespace)::operator<<(swift::Demangle::DemanglerPrinter&, \
             (anonymous namespace)::QuotedString const&)"
        ),
        "operator<<"
    );
    // Written by hand: the binary has no `>>=` or `new[]` of its own.
    assert_eq!(short_name("Foo::operator>>=(int)"), "Foo::operator>>=");
    assert_eq!(
        short_name("Foo::operator new[](unsigned long)"),
        "Foo::operator new[]"
    );
    // Not an operator: a word that merely ends in one.
    assert_eq!(
        short_name("skia::my_operator<int>::run()"),
        "my_operator::run"
    );
}

/// A C++ return type and an MSVC access prefix sit in front of the first segment, where
/// the name is the last word rather than the first.
#[test]
fn a_return_type_in_front_of_the_name_is_not_the_name() {
    assert_eq!(
        short_name(
            "void std::deque<skia::textlayout::OneLineShaper::RunBlock, \
             std::allocator<skia::textlayout::OneLineShaper::RunBlock> >\
             ::_M_push_back_aux<skia::textlayout::OneLineShaper::RunBlock&>\
             (skia::textlayout::OneLineShaper::RunBlock&)"
        ),
        "deque::_M_push_back_aux"
    );
    assert_eq!(
        short_name(
            "bool GrTTopoSort_Visit<GrRenderTask, GrRenderTask::TopoSortTraits>\
             (GrRenderTask*, unsigned int*)"
        ),
        "GrTTopoSort_Visit"
    );
    // Written by hand: this binary is not an MSVC one.
    assert_eq!(
        short_name("public: void __cdecl Foo::bar(void)"),
        "Foo::bar"
    );
}

/// An anonymous namespace is a segment that names nothing, and a name it is the whole of
/// is better drawn as the one segment that is left.
#[test]
fn an_anonymous_namespace_is_not_a_segment() {
    assert_eq!(
        short_name(
            "(anonymous namespace)::GaussianPass<unsigned char>::MakeMaker(float, SkArenaAlloc*)\
             ::Maker::bufferSizeBytes() const"
        ),
        "Maker::bufferSizeBytes"
    );
}

/// The `>` of a `->` closes nothing, and an `extern "C"` puts a quoted run inside the
/// arguments. Read either wrong and the group ends early, taking the function with it.
#[test]
fn a_function_type_inside_the_arguments_does_not_end_them() {
    assert_eq!(
        short_name(
            "<libloading::safe::Library>::get::\
             <unsafe extern \"C\" fn(*mut xkbcommon_dl::xkb_keymap, \
             *mut core::ffi::c_void, i32) -> *mut xkbcommon_dl::xkb_state>"
        ),
        "Library::get"
    );
}

/// A name no demangler would take is drawn exactly as the file states it, the way the
/// rest of the app draws one.
#[test]
fn a_name_nothing_can_be_made_of_comes_back_whole() {
    let mangled =
        "_ZN4core3fmt3num52_$LT$impl$u20$core..fmt..LowerHex$u20$for$u20$i32$GT$3fmt17hf00dE";
    assert_eq!(short_name(mangled), mangled);
    assert_eq!(short_name(""), "");
    assert_eq!(short_name("::"), "::");
}

/// Never panic on any file input: a name is whatever bytes the symbol table holds, and
/// nothing here is allowed to index its way out of one.
#[test]
fn a_name_that_makes_no_sense_is_answered_rather_than_panicked_on() {
    for name in [
        "<<<<<<",
        ">>>>>>",
        "((((((",
        "a::<",
        "a::b<c",
        "operator",
        "operator<",
        "  ::  ",
        "{{closure}}",
        "{",
        "<>",
        "<A as >",
        " as ",
        "\"",
        "a::\"unterminated",
        "→::λ<Ω>::漢字",
        "<漢字 as 字>::λ",
        "operator→",
    ] {
        // Whatever comes back, it came back.
        assert!(!short_name(name).is_empty() || name.trim().is_empty());
    }
}
