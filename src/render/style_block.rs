//! Emit the `<style>` block that ships default CSS variables alongside the SVG.
//!
//! Wrapped in `@layer plume.defaults` per SPEC §11.3 so any unlayered host CSS
//! (a runtime theme switcher, an embedding page) wins automatically.

use super::values::format_value;
use crate::resolve::{VarKind, VarTable};
use crate::Options;
use std::fmt::Write;

pub fn emit(out: &mut String, vars: &VarTable, opts: &Options) {
    if opts.bake_vars || opts.no_defaults {
        // `--bake-vars` inlines every value, and `--no-defaults` defers the
        // defaults to the host page. Either way, skip the `<style>` block.
        return;
    }

    // Only visual vars are exposed as CSS-themable defaults. Layout vars are
    // baked into the output — they're language constants, not theming hooks.
    let mut names: Vec<&String> = vars
        .entries
        .iter()
        .filter(|(_, e)| e.kind == VarKind::Visual)
        .map(|(n, _)| n)
        .collect();
    names.sort();

    if names.is_empty() {
        return;
    }

    out.push_str("  <style>@layer plume.defaults { :root, .plume {");
    for (i, name) in names.iter().enumerate() {
        let entry = vars.entries.get(*name).unwrap();
        if i > 0 {
            out.push(' ');
        }
        write!(
            out,
            " --plume-{}: {};",
            name,
            format_value(&entry.value, vars, opts)
        )
        .unwrap();
    }
    out.push_str(" } }</style>\n");
}
