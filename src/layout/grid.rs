//! Grid layout.
//!
//! Supports `cols=N` (with optional `rows=N`), explicit `col=` / `row=` on
//! children (1-indexed), declaration-order auto-flow for un-placed children,
//! and `col-widths` / `row-heights` (scalar or list). `colspan` and `rowspan`
//! default to 1.

use super::ir::{Bbox, PlacedNode};
use super::primitives;
use super::values::as_number_tuple;
use crate::error::Error;
use crate::resolve::{AttrMap, ResolvedValue, VarTable};
use crate::span::Span;

pub fn lay_out_grid(
    children: &mut [PlacedNode],
    attrs: &AttrMap,
    vars: &VarTable,
    span: Span,
) -> Result<Bbox, Error> {
    let cols = attr_uint(attrs, "cols", span)?;
    let rows = attr_uint(attrs, "rows", span)?;

    // Survey explicit child placements so auto-derived track counts grow to
    // fit them — `col=3 row=2` on a child shouldn't error just because the
    // container didn't list `rows=2`.
    let mut max_col_used: usize = 0;
    let mut max_row_used: usize = 0;
    for child in children.iter() {
        let cs = attr_uint(&child.attrs, "colspan", child.span)?
            .unwrap_or(1)
            .max(1);
        let rs = attr_uint(&child.attrs, "rowspan", child.span)?
            .unwrap_or(1)
            .max(1);
        if let Some(c) = attr_uint(&child.attrs, "col", child.span)? {
            max_col_used = max_col_used.max(c.saturating_sub(1) + cs);
        }
        if let Some(r) = attr_uint(&child.attrs, "row", child.span)? {
            max_row_used = max_row_used.max(r.saturating_sub(1) + rs);
        }
    }

    let (cols, rows) = match (cols, rows) {
        (Some(c), Some(r)) => (c, r),
        (Some(c), None) => {
            let auto_r = children.len().div_ceil(c.max(1));
            (c, auto_r.max(max_row_used).max(1))
        }
        (None, Some(r)) => {
            let auto_c = children.len().div_ceil(r.max(1));
            (auto_c.max(max_col_used).max(1), r)
        }
        (None, None) => {
            return Err(Error::at(span, "grid layout requires 'cols' or 'rows'"));
        }
    };

    let (gap_y, gap_x) = primitives::gap(attrs, vars, span)?;

    // Track sizes: explicit col-widths / row-heights, else auto from children.
    let explicit_col = read_track_sizes(attrs, "col-widths", cols, span)?;
    let explicit_row = read_track_sizes(attrs, "row-heights", rows, span)?;

    // Assign positions: build a 2D occupancy map.
    let mut placements: Vec<Placement> = Vec::with_capacity(children.len());
    let mut occupied = vec![vec![false; cols]; rows];

    for (i, child) in children.iter().enumerate() {
        let cs = attr_uint(&child.attrs, "colspan", child.span)?
            .unwrap_or(1)
            .max(1);
        let rs = attr_uint(&child.attrs, "rowspan", child.span)?
            .unwrap_or(1)
            .max(1);
        let explicit_col_idx = attr_uint(&child.attrs, "col", child.span)?;
        let explicit_row_idx = attr_uint(&child.attrs, "row", child.span)?;

        let (col, row) = match (explicit_col_idx, explicit_row_idx) {
            (Some(c), Some(r)) => (c.saturating_sub(1), r.saturating_sub(1)),
            (Some(c), None) => {
                let c = c.saturating_sub(1);
                let r = find_row_for(c, cs, &occupied, rows);
                (c, r)
            }
            (None, Some(r)) => {
                let r = r.saturating_sub(1);
                let c = find_col_for(r, cs, &occupied, cols);
                (c, r)
            }
            (None, None) => next_open(cs, rs, &occupied, cols, rows).unwrap_or((0, 0)),
        };

        if col + cs > cols || row + rs > rows {
            return Err(Error::at(
                child.span,
                format!("col={} (span {}) exceeds cols={}", col + 1, cs, cols),
            ));
        }

        for dr in 0..rs {
            for dc in 0..cs {
                occupied[row + dr][col + dc] = true;
            }
        }
        placements.push(Placement {
            child_index: i,
            col,
            row,
            colspan: cs,
            rowspan: rs,
        });
    }

    // Compute auto-sized tracks (max child size per track, considering spans
    // only when they distribute evenly — Sprint 3 keeps that simple).
    let mut col_widths = explicit_col.clone().unwrap_or_else(|| vec![0.0_f64; cols]);
    let mut row_heights = explicit_row.clone().unwrap_or_else(|| vec![0.0_f64; rows]);
    if explicit_col.is_none() {
        for p in &placements {
            if p.colspan == 1 {
                col_widths[p.col] = col_widths[p.col].max(children[p.child_index].bbox.w());
            }
        }
    }
    if explicit_row.is_none() {
        for p in &placements {
            if p.rowspan == 1 {
                row_heights[p.row] = row_heights[p.row].max(children[p.child_index].bbox.h());
            }
        }
    }

    // Cumulative offsets per track.
    let col_offsets = cumulative(&col_widths, gap_x);
    let row_offsets = cumulative(&row_heights, gap_y);

    let total_w = col_offsets[cols] - gap_x;
    let total_h = row_offsets[rows] - gap_y;

    // Place each child centered in its (possibly spanning) cell.
    for p in &placements {
        let cell_x_start = col_offsets[p.col];
        let cell_y_start = row_offsets[p.row];
        let cell_x_end = col_offsets[p.col + p.colspan] - gap_x;
        let cell_y_end = row_offsets[p.row + p.rowspan] - gap_y;
        let cell_cx = (cell_x_start + cell_x_end) / 2.0 - total_w / 2.0;
        let cell_cy = (cell_y_start + cell_y_end) / 2.0 - total_h / 2.0;

        let child = &mut children[p.child_index];
        let local_offset_x = (child.bbox.min_x + child.bbox.max_x) / 2.0;
        let local_offset_y = (child.bbox.min_y + child.bbox.max_y) / 2.0;
        child.cx = cell_cx - local_offset_x;
        child.cy = cell_cy - local_offset_y;
    }

    Ok(Bbox::centered(total_w, total_h))
}

struct Placement {
    child_index: usize,
    col: usize,
    row: usize,
    colspan: usize,
    rowspan: usize,
}

fn read_track_sizes(
    attrs: &AttrMap,
    name: &str,
    track_count: usize,
    span: Span,
) -> Result<Option<Vec<f64>>, Error> {
    match attrs.get(name) {
        Some(ResolvedValue::Number(n)) => Ok(Some(vec![*n; track_count])),
        Some(ResolvedValue::List(items)) => {
            if items.len() != track_count {
                return Err(Error::at(
                    span,
                    format!(
                        "'{}' has {} values but {}={}",
                        name,
                        items.len(),
                        if name == "col-widths" { "cols" } else { "rows" },
                        track_count
                    ),
                ));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(super::values::as_number(item, span)?);
            }
            Ok(Some(out))
        }
        Some(other) => {
            // Allow tuple form too.
            let nums = as_number_tuple(other, span)?;
            if nums.len() != track_count {
                return Err(Error::at(
                    span,
                    format!(
                        "'{}' has {} values but {}={}",
                        name,
                        nums.len(),
                        if name == "col-widths" { "cols" } else { "rows" },
                        track_count
                    ),
                ));
            }
            Ok(Some(nums))
        }
        None => Ok(None),
    }
}

fn attr_uint(attrs: &AttrMap, name: &str, span: Span) -> Result<Option<usize>, Error> {
    match attrs.get(name) {
        Some(v) => {
            let n = super::values::as_number(v, span)?;
            if n < 0.0 || n.fract() != 0.0 {
                return Err(Error::at(
                    span,
                    format!("'{}' expects a non-negative integer, got {}", name, n),
                ));
            }
            Ok(Some(n as usize))
        }
        None => Ok(None),
    }
}

fn cumulative(sizes: &[f64], gap: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(sizes.len() + 1);
    let mut acc = 0.0;
    out.push(acc);
    for s in sizes {
        acc += s + gap;
        out.push(acc);
    }
    out
}

fn find_row_for(col: usize, cs: usize, occupied: &[Vec<bool>], _rows: usize) -> usize {
    for (r, row) in occupied.iter().enumerate() {
        if (0..cs).all(|dc| col + dc < row.len() && !row[col + dc]) {
            return r;
        }
    }
    0
}

fn find_col_for(row: usize, cs: usize, occupied: &[Vec<bool>], cols: usize) -> usize {
    for c in 0..cols.saturating_sub(cs.saturating_sub(1)) {
        if (0..cs).all(|dc| !occupied[row][c + dc]) {
            return c;
        }
    }
    0
}

fn next_open(
    cs: usize,
    rs: usize,
    occupied: &[Vec<bool>],
    cols: usize,
    rows: usize,
) -> Option<(usize, usize)> {
    for r in 0..rows.saturating_sub(rs.saturating_sub(1)) {
        for c in 0..cols.saturating_sub(cs.saturating_sub(1)) {
            let free = (0..rs).all(|dr| (0..cs).all(|dc| !occupied[r + dr][c + dc]));
            if free {
                return Some((c, r));
            }
        }
    }
    None
}
