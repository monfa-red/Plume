use super::ir::{ResolvedCall, ResolvedValue, VarEntry, VarKind, VarTable};
use crate::ast::{DefaultEntry, FnCall, Value};
use crate::error::Error;

/// Built-in CSS variable defaults per SPEC §11.1. Names are stored without the
/// `--plume-` prefix.
pub fn built_in_defaults() -> VarTable {
    let mut t = VarTable::new();

    // Visual vars — live at runtime.
    set_visual(&mut t, "bg", ResolvedValue::Ident("white".into()));
    set_visual(&mut t, "fg", ResolvedValue::Hex("222".into()));
    set_visual(&mut t, "fill", ResolvedValue::Ident("white".into()));
    set_visual(&mut t, "stroke", ResolvedValue::Hex("444".into()));
    set_visual(&mut t, "accent", ResolvedValue::Hex("0a84ff".into()));
    set_visual(&mut t, "on-accent", ResolvedValue::Ident("white".into()));
    set_visual(&mut t, "muted", ResolvedValue::Hex("888".into()));
    set_visual(&mut t, "danger", ResolvedValue::Ident("crimson".into()));
    set_visual(&mut t, "warn", ResolvedValue::Ident("orange".into()));
    set_visual(&mut t, "note-bg", ResolvedValue::Hex("fff9c4".into()));
    set_visual(
        &mut t,
        "font",
        ResolvedValue::String("system-ui, -apple-system, sans-serif".into()),
    );
    // text-color defaults to var(--plume-fg).
    set_visual(
        &mut t,
        "text-color",
        ResolvedValue::LiveVar {
            name: "fg".into(),
            raw: false,
            baked: None,
        },
    );
    set_visual(
        &mut t,
        "shadow",
        ResolvedValue::Call(ResolvedCall {
            name: "rgba".into(),
            args: vec![
                ResolvedValue::Number(0.0),
                ResolvedValue::Number(0.0),
                ResolvedValue::Number(0.0),
                ResolvedValue::Number(0.2),
            ],
        }),
    );

    // Layout vars — baked at compile time.
    set_layout_n(&mut t, "text-size", 13.0);
    set_layout_n(&mut t, "text-pad", 16.0);
    set_layout_n(&mut t, "gap", 20.0);
    set_layout_n(&mut t, "padding", 0.0);
    set_layout_n(&mut t, "thickness", 1.0);
    set_layout_n(&mut t, "radius", 0.0);
    set_layout_n(&mut t, "rect-w", 100.0);
    set_layout_n(&mut t, "rect-h", 40.0);
    set_layout_n(&mut t, "oval-rx", 30.0);
    set_layout_n(&mut t, "oval-ry", 20.0);
    set_layout_n(&mut t, "circle-r", 20.0);
    set_layout_n(&mut t, "arrow-head", 10.0);
    set_layout_n(&mut t, "icon-size", 24.0);
    set_layout_n(&mut t, "canvas-pad", 20.0);

    t
}

fn set_layout_n(t: &mut VarTable, name: &str, n: f64) {
    t.set(name, VarKind::Layout, ResolvedValue::Number(n));
}

fn set_visual(t: &mut VarTable, name: &str, v: ResolvedValue) {
    t.set(name, VarKind::Visual, v);
}

/// Apply a `defaults {}` block on top of the table. Each entry overrides the
/// previous value; unknown names are introduced as Visual vars so user-defined
/// `--plume-*` vars can be themed at runtime.
pub fn apply_defaults_block(table: &mut VarTable, entries: &[DefaultEntry]) -> Result<(), Error> {
    for entry in entries {
        let value = resolve_value(&entry.value, table)?;
        let kind = match table.get(&entry.name) {
            Some(VarEntry { kind, .. }) => *kind,
            None => VarKind::Visual,
        };
        table.set(entry.name.clone(), kind, value);
    }
    Ok(())
}

/// Resolve a syntactic `Value` from the AST into a `ResolvedValue`. The only
/// transformation is `var()` → `LiveVar` with baked layout values where the
/// referenced var has VarKind::Layout.
pub fn resolve_value(value: &Value, table: &VarTable) -> Result<ResolvedValue, Error> {
    Ok(match value {
        Value::Number(n) => ResolvedValue::Number(*n),
        Value::String(s) => ResolvedValue::String(s.clone()),
        Value::Hex(h) => ResolvedValue::Hex(h.clone()),
        Value::Ident(s) => ResolvedValue::Ident(s.clone()),
        Value::Tuple(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_value(item, table)?);
            }
            ResolvedValue::Tuple(out)
        }
        Value::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_value(item, table)?);
            }
            ResolvedValue::List(out)
        }
        Value::Call(call) => resolve_call(call, table)?,
        Value::RawCssVar(_) => {
            unreachable!("parser rejects raw css var outside var()");
        }
    })
}

fn resolve_call(call: &FnCall, table: &VarTable) -> Result<ResolvedValue, Error> {
    if call.name == "var" {
        if call.args.len() != 1 {
            return Err(Error::at(
                call.span,
                format!("var() expects 1 argument, got {}", call.args.len()),
            ));
        }
        match &call.args[0] {
            Value::Ident(name) => {
                let baked = match table.get(name) {
                    Some(VarEntry {
                        kind: VarKind::Layout,
                        value,
                    }) => Some(Box::new(value.clone())),
                    _ => None,
                };
                Ok(ResolvedValue::LiveVar {
                    name: name.clone(),
                    raw: false,
                    baked,
                })
            }
            Value::RawCssVar(name) => Ok(ResolvedValue::LiveVar {
                name: name.clone(),
                raw: true,
                baked: None,
            }),
            _ => Err(Error::at(
                call.span,
                "var() argument must be an identifier or --css-var",
            )),
        }
    } else {
        let mut args = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            args.push(resolve_value(arg, table)?);
        }
        Ok(ResolvedValue::Call(ResolvedCall {
            name: call.name.clone(),
            args,
        }))
    }
}
