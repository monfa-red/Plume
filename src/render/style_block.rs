//! Emit the `<style>` block that ships default CSS variables alongside the SVG.
//!
//! Wrapped in `@layer plume.defaults` per SPEC §11.3 so any unlayered host CSS
//! (a runtime theme switcher, an embedding page) wins automatically.

use super::values::format_value;
use super::Options;
use crate::resolve::VarTable;
use std::fmt::Write;

pub fn emit(out: &mut String, vars: &VarTable, opts: &Options) {
    if opts.bake_vars {
        // Bake mode: every `var()` has been replaced with its literal in the
        // tree, so no defaults block is needed (and including one would only
        // change the renderer if it does support vars — which is the opposite
        // of what bake mode is for).
        return;
    }

    let mut names: Vec<&String> = vars.entries.keys().collect();
    names.sort();

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
