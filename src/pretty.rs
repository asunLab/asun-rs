//! Pretty-printed ASUN text — smart indentation over the compact encoders.
//!
//! [`encode_pretty`] / [`encode_pretty_typed`] encode a value and then reflow it;
//! [`pretty_format`] reflows already-encoded compact ASUN bytes.
//!
//! Simple structures stay inline:
//!
//! ```text
//! {name@str, age@int}:(Alice, 30)
//! ```
//!
//! Complex structures expand with 2-space indentation:
//!
//! ```text
//! {
//!   id@str,
//!   name@str,
//!   addr@{city@str, zip@int}
//! }:
//!   (E001, John, (NYC, 10001))
//! ```

use crate::error::Result;
use crate::traits::AsunEncode;

const PRETTY_MAX_WIDTH: usize = 100;

/// Serialize a struct to pretty-formatted ASUN string.
pub fn encode_pretty<T: AsunEncode + ?Sized>(value: &T) -> Result<String> {
    let compact = crate::encode::encode(value)?;
    Ok(pretty_format(compact.as_bytes()))
}

/// Serialize a struct to pretty-formatted ASUN string with type annotations.
pub fn encode_pretty_typed<T: AsunEncode + ?Sized>(value: &T) -> Result<String> {
    let compact = crate::encode::encode_typed(value)?;
    Ok(pretty_format(compact.as_bytes()))
}

/// Reformat compact ASUN bytes with smart indentation.
pub fn pretty_format(src: &[u8]) -> String {
    let n = src.len();
    if n == 0 {
        return String::new();
    }

    let mat = build_match_table(src);
    let mut f = PrettyFmt {
        src,
        mat: &mat,
        out: Vec::with_capacity(n * 2),
        pos: 0,
        depth: 0,
    };
    f.write_top();
    unsafe { String::from_utf8_unchecked(f.out) }
}

/// Length of the token at `i` whose bytes are never structural and must be
/// copied verbatim: a quoted string, a `\x` escape inside a plain string, or a
/// comment. `0` when `src[i]` starts none of them. Unterminated tokens run to
/// the end of the input.
#[inline]
fn atom_len(src: &[u8], i: usize) -> usize {
    let n = src.len();
    match src[i] {
        b'"' => {
            let mut j = i + 1;
            while j < n {
                match src[j] {
                    b'\\' => j += 2,
                    b'"' => return j + 1 - i,
                    _ => j += 1,
                }
            }
            n - i
        }
        b'\\' => (n - i).min(2),
        b'/' if i + 1 < n && src[i + 1] == b'*' => {
            let mut j = i + 2;
            while j + 1 < n {
                if src[j] == b'*' && src[j + 1] == b'/' {
                    return j + 2 - i;
                }
                j += 1;
            }
            n - i
        }
        _ => 0,
    }
}

fn build_match_table(src: &[u8]) -> Vec<i32> {
    let n = src.len();
    let mut mat = vec![-1i32; n];
    let mut stack: Vec<usize> = Vec::with_capacity(32);
    let mut i = 0;
    while i < n {
        let atom = atom_len(src, i);
        if atom > 0 {
            i += atom;
            continue;
        }
        match src[i] {
            b'{' | b'(' | b'[' => stack.push(i),
            b'}' | b')' | b']' => {
                if let Some(j) = stack.pop() {
                    mat[j] = i as i32;
                    mat[i] = j as i32;
                }
            }
            _ => {}
        }
        i += 1;
    }
    mat
}

struct PrettyFmt<'a> {
    src: &'a [u8],
    mat: &'a [i32],
    out: Vec<u8>,
    pos: usize,
    depth: usize,
}

impl<'a> PrettyFmt<'a> {
    fn write_top(&mut self) {
        if self.pos >= self.src.len() {
            return;
        }
        if self.src[self.pos] == b'['
            && self.pos + 1 < self.src.len()
            && self.src[self.pos + 1] == b'{'
        {
            self.write_array_top();
        } else if self.src[self.pos] == b'{' {
            self.write_object_top();
        } else {
            self.out.extend_from_slice(&self.src[self.pos..]);
        }
    }

    fn write_object_top(&mut self) {
        self.write_group();
        if self.pos < self.src.len() && self.src[self.pos] == b':' {
            self.out.push(b':');
            self.pos += 1;
            if self.pos < self.src.len() {
                let close = self.mat[self.pos];
                if close >= 0 && (close as usize) - self.pos < PRETTY_MAX_WIDTH {
                    let end = close as usize + 1;
                    self.write_inline(self.pos, end);
                    self.pos = end;
                } else {
                    self.out.push(b'\n');
                    self.depth += 1;
                    self.write_indent();
                    self.write_group();
                    self.depth -= 1;
                }
            }
        }
    }

    fn write_array_top(&mut self) {
        self.out.push(b'[');
        self.pos += 1;
        self.write_group();
        if self.pos < self.src.len() && self.src[self.pos] == b']' {
            self.out.push(b']');
            self.pos += 1;
        }
        if self.pos < self.src.len() && self.src[self.pos] == b':' {
            self.out.extend_from_slice(b":\n");
            self.pos += 1;
        }

        self.depth += 1;
        let mut first = true;
        while self.pos < self.src.len() {
            if self.src[self.pos] == b',' {
                self.pos += 1;
            }
            if self.pos >= self.src.len() {
                break;
            }
            if !first {
                self.out.extend_from_slice(b",\n");
            }
            first = false;
            self.write_indent();
            let before = self.pos;
            self.write_group();
            if self.pos == before {
                // A stray closer after the rows: copy it so the loop advances.
                self.out.push(self.src[self.pos]);
                self.pos += 1;
            }
        }
        self.out.push(b'\n');
        self.depth -= 1;
    }

    fn write_group(&mut self) {
        if self.pos >= self.src.len() {
            return;
        }
        let ch = self.src[self.pos];
        if ch != b'{' && ch != b'(' && ch != b'[' {
            self.write_value();
            return;
        }

        // Special case: [{...}] array schema — fuse brackets
        if ch == b'[' && self.pos + 1 < self.src.len() && self.src[self.pos + 1] == b'{' {
            let close_brace = self.mat[self.pos + 1];
            let close_bracket = self.mat[self.pos];
            if close_brace >= 0 && close_bracket >= 0 && close_brace + 1 == close_bracket {
                let width = close_bracket as usize - self.pos + 1;
                if width <= PRETTY_MAX_WIDTH {
                    let end = close_bracket as usize + 1;
                    self.write_inline(self.pos, end);
                    self.pos = end;
                    return;
                }
                self.out.push(b'[');
                self.pos += 1;
                self.write_group();
                self.out.push(b']');
                self.pos += 1;
                return;
            }
        }

        let close_pos = self.mat[self.pos];
        if close_pos < 0 {
            self.out.push(ch);
            self.pos += 1;
            return;
        }
        let close = close_pos as usize;
        let width = close - self.pos + 1;
        if width <= PRETTY_MAX_WIDTH {
            self.write_inline(self.pos, close + 1);
            self.pos = close + 1;
            return;
        }

        // Expanded form
        let close_ch = self.src[close];
        self.out.push(ch);
        self.pos += 1;

        if self.pos >= close {
            self.out.push(close_ch);
            self.pos = close + 1;
            return;
        }

        self.out.push(b'\n');
        self.depth += 1;

        let mut first = true;
        while self.pos < close {
            // The comma is consumed only between slots: a leading `,` is the
            // end of an empty (null) first slot and must be kept.
            if !first && self.src[self.pos] == b',' {
                self.pos += 1;
            }
            if !first {
                self.out.extend_from_slice(b",\n");
            }
            first = false;
            self.write_indent();
            self.write_element(close);
        }

        self.out.push(b'\n');
        self.depth -= 1;
        self.write_indent();
        self.out.push(close_ch);
        self.pos = close + 1;
    }

    fn write_element(&mut self, boundary: usize) {
        while self.pos < boundary && self.src[self.pos] != b',' {
            let ch = self.src[self.pos];
            if ch == b'{' || ch == b'(' || ch == b'[' {
                self.write_group();
            } else if atom_len(self.src, self.pos) > 0 {
                self.write_atom();
            } else {
                self.out.push(ch);
                self.pos += 1;
            }
        }
    }

    fn write_value(&mut self) {
        while self.pos < self.src.len() {
            let ch = self.src[self.pos];
            if ch == b',' || ch == b')' || ch == b'}' || ch == b']' {
                break;
            }
            if atom_len(self.src, self.pos) > 0 {
                self.write_atom();
            } else {
                self.out.push(ch);
                self.pos += 1;
            }
        }
    }

    fn write_atom(&mut self) {
        let end = self.pos + atom_len(self.src, self.pos);
        self.out.extend_from_slice(&self.src[self.pos..end]);
        self.pos = end;
    }

    fn write_inline(&mut self, start: usize, end: usize) {
        let mut depth: i32 = 0;
        let mut i = start;
        while i < end {
            let ch = self.src[i];
            let atom = atom_len(self.src, i).min(end - i);
            if atom > 0 {
                self.out.extend_from_slice(&self.src[i..i + atom]);
                i += atom;
                continue;
            }
            match ch {
                b'{' | b'(' | b'[' => {
                    depth += 1;
                    self.out.push(ch);
                }
                b'}' | b')' | b']' => {
                    depth -= 1;
                    self.out.push(ch);
                }
                b',' => {
                    self.out.push(b',');
                    if depth == 1 {
                        self.out.push(b' ');
                    }
                }
                _ => self.out.push(ch),
            }
            i += 1;
        }
    }

    fn write_indent(&mut self) {
        for _ in 0..self.depth {
            self.out.extend_from_slice(b"  ");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pretty_format;

    #[test]
    fn stray_closer_after_rows_terminates() {
        // Used to loop forever, growing the output without bound.
        let out = pretty_format(b"[{a}]:(1))");
        assert_eq!(out, "[{a}]:\n  (1),\n  )\n");
    }
}
