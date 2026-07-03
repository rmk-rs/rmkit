//! Render a decoded layout variant as Unicode box-drawing text.
//!
//! Key-units map onto a fixed character grid (1u = 6 columns × 3 rows, roughly
//! square in a terminal). Every box edge is drawn into a per-cell direction
//! bitmask, so borders shared between keys merge into proper `┬`/`├`/`┼`
//! junctions. Labels overlay the interior afterwards. A character grid cannot
//! draw rotation: rotated keys render axis-aligned at their position with the
//! angle printed inside.

use rynk_kle::layout::Variant;

/// Character cells per key-unit.
const SX: f32 = 6.0;
const SY: f32 = 3.0;

/// Per-cell line-direction bits; `GLYPHS` is indexed by their combination.
const N: u8 = 1;
const S: u8 = 2;
const E: u8 = 4;
const W: u8 = 8;

const GLYPHS: [char; 16] = [
    ' ', '╵', '╷', '│', '╶', '└', '┌', '├', '╴', '┘', '┐', '┤', '─', '┴', '┬', '┼',
];

/// A rectangle in character-cell coordinates, borders inclusive.
#[derive(Clone, Copy)]
struct CellBox {
    r0: i32,
    c0: i32,
    r1: i32,
    c1: i32,
}

impl CellBox {
    /// From a center + size in key-units; `(ox, oy)` is the canvas origin.
    fn new(x: f32, y: f32, w: f32, h: f32, ox: f32, oy: f32) -> Self {
        let c0 = ((x - w / 2.0 - ox) * SX).round() as i32;
        let r0 = ((y - h / 2.0 - oy) * SY).round() as i32;
        // Clamp to at least one cell so a degenerate shape still draws a box.
        let c1 = (((x + w / 2.0 - ox) * SX).round() as i32).max(c0 + 1);
        let r1 = (((y + h / 2.0 - oy) * SY).round() as i32).max(r0 + 1);
        CellBox { r0, c0, r1, c1 }
    }

    fn strictly_contains(&self, r: i32, c: i32) -> bool {
        r > self.r0 && r < self.r1 && c > self.c0 && c < self.c1
    }
}

/// One drawable item: a key (optionally two rectangles) or an encoder.
struct Item {
    b: CellBox,
    b2: Option<CellBox>,
    round: bool,
    label: Vec<String>,
}

struct Canvas {
    w: usize,
    h: usize,
    line: Vec<u8>,
    round: Vec<bool>,
    text: Vec<char>,
}

impl Canvas {
    fn set(&mut self, r: i32, c: i32, bits: u8, round: bool) {
        if (0..self.h as i32).contains(&r) && (0..self.w as i32).contains(&c) {
            let i = r as usize * self.w + c as usize;
            self.line[i] |= bits;
            self.round[i] |= round;
        }
    }

    /// Erase line bits strictly inside `b`: keys that physically overlap (e.g.
    /// rotated thumbs drawn axis-aligned) occlude what's under them, painter's
    /// style, while shared borders still merge into junctions.
    fn clear_inside(&mut self, b: CellBox) {
        for r in b.r0 + 1..b.r1 {
            for c in b.c0 + 1..b.c1 {
                if (0..self.h as i32).contains(&r) && (0..self.w as i32).contains(&c) {
                    let i = r as usize * self.w + c as usize;
                    self.line[i] = 0;
                    self.round[i] = false;
                }
            }
        }
    }

    /// Draw a box outline. `skip` erases cells interior to the sibling rect of
    /// a two-rect key, so the pair renders as its union silhouette.
    fn draw_box(&mut self, b: CellBox, round: bool, skip: impl Fn(i32, i32) -> bool) {
        for c in b.c0..=b.c1 {
            let mut bits = 0;
            if c > b.c0 {
                bits |= W;
            }
            if c < b.c1 {
                bits |= E;
            }
            let end = c == b.c0 || c == b.c1;
            if !skip(b.r0, c) {
                self.set(b.r0, c, bits | if end { S } else { 0 }, round);
            }
            if !skip(b.r1, c) {
                self.set(b.r1, c, bits | if end { N } else { 0 }, round);
            }
        }
        for r in b.r0 + 1..b.r1 {
            if !skip(r, b.c0) {
                self.set(r, b.c0, N | S, round);
            }
            if !skip(r, b.c1) {
                self.set(r, b.c1, N | S, round);
            }
        }
    }

    /// Center `lines` in the interior of `b`, clipped to fit.
    fn label(&mut self, b: CellBox, lines: &[String]) {
        let iw = (b.c1 - b.c0 - 1).max(0) as usize;
        let ih = (b.r1 - b.r0 - 1).max(0) as usize;
        let n = lines.len().min(ih);
        let r_start = b.r0 + 1 + ((ih - n) / 2) as i32;
        for (i, line) in lines[..n].iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let len = chars.len().min(iw);
            let c_start = b.c0 + 1 + ((iw - len) / 2) as i32;
            for (j, &ch) in chars[..len].iter().enumerate() {
                let (r, c) = (r_start + i as i32, c_start + j as i32);
                if (0..self.h as i32).contains(&r) && (0..self.w as i32).contains(&c) {
                    self.text[r as usize * self.w + c as usize] = ch;
                }
            }
        }
    }
}

/// Render one variant: a one-line stats header, a blank line, then the art.
pub fn render_variant(v: &Variant, is_default: bool) -> String {
    let mut header = format!("variant '{}'", v.name);
    if is_default {
        header.push_str(" (default)");
    }
    if v.keys.is_empty() && v.encoders.is_empty() {
        return format!("{header}: empty (all keys hidden)\n");
    }

    // Bounds over every rectangle, in key-units.
    let mut min = (f32::MAX, f32::MAX);
    let mut max = (f32::MIN, f32::MIN);
    let mut grow = |x: f32, y: f32, w: f32, h: f32| {
        min.0 = min.0.min(x - w / 2.0);
        min.1 = min.1.min(y - h / 2.0);
        max.0 = max.0.max(x + w / 2.0);
        max.1 = max.1.max(y + h / 2.0);
    };
    for k in &v.keys {
        grow(k.x, k.y, k.w, k.h);
        if let Some(r2) = &k.rect2 {
            grow(r2.x, r2.y, r2.w, r2.h);
        }
    }
    for e in &v.encoders {
        grow(e.x, e.y, e.w, e.h);
    }

    let n = v.keys.len();
    header.push_str(&format!(": {n} key{}", if n == 1 { "" } else { "s" }));
    let n = v.encoders.len();
    if n > 0 {
        header.push_str(&format!(", {n} encoder{}", if n == 1 { "" } else { "s" }));
    }
    // Snap the size to 0.01u so f32 walk noise doesn't print as 4.0500002.
    header.push_str(&format!(
        ", {}u × {}u",
        ((max.0 - min.0) * 100.0).round() / 100.0,
        ((max.1 - min.1) * 100.0).round() / 100.0
    ));

    let items: Vec<Item> = v
        .keys
        .iter()
        .map(|k| {
            let mut label = vec![format!("{},{}", k.row, k.col)];
            if k.r != 0.0 {
                label.push(format!("{}°", k.r));
            }
            Item {
                b: CellBox::new(k.x, k.y, k.w, k.h, min.0, min.1),
                b2: k
                    .rect2
                    .as_ref()
                    .map(|r2| CellBox::new(r2.x, r2.y, r2.w, r2.h, min.0, min.1)),
                round: false,
                label,
            }
        })
        .chain(v.encoders.iter().map(|e| {
            let mut label = vec![format!("E{}", e.id)];
            if e.r != 0.0 {
                label.push(format!("{}°", e.r));
            }
            // Rotation arrows mark the knob as rotary; on a 1u knob they fit
            // only when no angle claims the second interior row (label() clips).
            label.push("↺ ↻".to_string());
            Item {
                b: CellBox::new(e.x, e.y, e.w, e.h, min.0, min.1),
                b2: None,
                round: true,
                label,
            }
        }))
        .collect();

    let (mut w, mut h) = (0i32, 0i32);
    for b in items.iter().flat_map(|i| [Some(i.b), i.b2]).flatten() {
        w = w.max(b.c1 + 1);
        h = h.max(b.r1 + 1);
    }
    let (w, h) = (w as usize, h as usize);
    let mut canvas = Canvas {
        w,
        h,
        line: vec![0; w * h],
        round: vec![false; w * h],
        text: vec!['\0'; w * h],
    };

    for it in &items {
        canvas.clear_inside(it.b);
        match it.b2 {
            None => canvas.draw_box(it.b, it.round, |_, _| false),
            Some(b2) => {
                canvas.clear_inside(b2);
                canvas.draw_box(it.b, it.round, |r, c| b2.strictly_contains(r, c));
                canvas.draw_box(b2, it.round, |r, c| it.b.strictly_contains(r, c));
            }
        }
    }
    // Occlusion chops edges mid-run, leaving direction bits that point at
    // nothing. Prune to a fixpoint: a bit survives only if the neighbor it
    // points at holds the reciprocal bit, so remnants end in clean junctions.
    loop {
        let snap = canvas.line.clone();
        let at = |r: i32, c: i32| -> u8 {
            if (0..h as i32).contains(&r) && (0..w as i32).contains(&c) {
                snap[r as usize * w + c as usize]
            } else {
                0
            }
        };
        let mut changed = false;
        for r in 0..h as i32 {
            for c in 0..w as i32 {
                let i = r as usize * w + c as usize;
                let mut bits = snap[i];
                if bits & N != 0 && at(r - 1, c) & S == 0 {
                    bits &= !N;
                }
                if bits & S != 0 && at(r + 1, c) & N == 0 {
                    bits &= !S;
                }
                if bits & E != 0 && at(r, c + 1) & W == 0 {
                    bits &= !E;
                }
                if bits & W != 0 && at(r, c - 1) & E == 0 {
                    bits &= !W;
                }
                if bits != snap[i] {
                    canvas.line[i] = bits;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Labels go on top of all lines.
    for it in &items {
        canvas.label(it.b, &it.label);
    }

    let mut out = header;
    out.push_str("\n\n");
    for r in 0..h {
        let mut row = String::with_capacity(w);
        for c in 0..w {
            let i = r * w + c;
            let ch = if canvas.text[i] != '\0' {
                canvas.text[i]
            } else {
                let bits = canvas.line[i];
                match bits {
                    b if canvas.round[i] && b == N | E => '╰',
                    b if canvas.round[i] && b == S | E => '╭',
                    b if canvas.round[i] && b == N | W => '╯',
                    b if canvas.round[i] && b == S | W => '╮',
                    b => GLYPHS[b as usize],
                }
            };
            row.push(ch);
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}
