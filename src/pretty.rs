// ---------------------------------------------------------------------------
// Pretty-print formatter — smart indentation for ASON output
// ---------------------------------------------------------------------------
//
// Simple structures stay inline:
//   {name:str, age:int}:(Alice, 30)
//
// Complex structures expand with 2-space indentation:
//   {
//     id:str,
//     name:str,
//     addr:{city:str, zip:int}
//   }:
//     (E001, John, (NYC, 10001))

use crate::error::Result;
use serde::Serialize;

const PRETTY_MAX_WIDTH: usize = 100;

/// Serialize a struct to pretty-formatted ASON string.
pub fn encode_pretty<T: Serialize>(value: &T) -> Result<String> {
    let compact = crate::encode::encode(value)?;
    Ok(pretty_format(compact.as_bytes()))
}

/// Serialize a struct to pretty-formatted ASON string with type annotations.
pub fn encode_pretty_typed<T: Serialize>(value: &T) -> Result<String> {
    let compact = crate::encode::encode_typed(value)?;
    Ok(pretty_format(compact.as_bytes()))
}

/// Reformat compact ASON bytes with smart indentation.
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

fn build_match_table(src: &[u8]) -> Vec<i32> {
    let n = src.len();
    let mut mat = vec![-1i32; n];
    let mut stack: Vec<usize> = Vec::with_capacity(32);
    let mut in_quote = false;
    let mut i = 0;
    while i < n {
        if in_quote {
            if src[i] == b'\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if src[i] == b'"' {
                in_quote = false;
            }
            i += 1;
            continue;
        }
        match src[i] {
            b'"' => in_quote = true,
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
        if self.src[self.pos] == b'[' && self.pos + 1 < self.src.len() && self.src[self.pos + 1] == b'{' {
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
                if close >= 0 && (close as usize) - self.pos + 1 <= PRETTY_MAX_WIDTH {
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
            self.write_group();
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
            if self.src[self.pos] == b',' {
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
            } else if ch == b'"' {
                self.write_quoted();
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
            if ch == b'"' {
                self.write_quoted();
            } else {
                self.out.push(ch);
                self.pos += 1;
            }
        }
    }

    fn write_quoted(&mut self) {
        self.out.push(b'"');
        self.pos += 1;
        while self.pos < self.src.len() {
            let ch = self.src[self.pos];
            self.out.push(ch);
            self.pos += 1;
            if ch == b'\\' && self.pos < self.src.len() {
                self.out.push(self.src[self.pos]);
                self.pos += 1;
            } else if ch == b'"' {
                break;
            }
        }
    }

    fn write_inline(&mut self, start: usize, end: usize) {
        let mut depth: i32 = 0;
        let mut in_quote = false;
        let mut i = start;
        while i < end {
            let ch = self.src[i];
            if in_quote {
                self.out.push(ch);
                if ch == b'\\' && i + 1 < end {
                    i += 1;
                    self.out.push(self.src[i]);
                } else if ch == b'"' {
                    in_quote = false;
                }
                i += 1;
                continue;
            }
            match ch {
                b'"' => {
                    in_quote = true;
                    self.out.push(ch);
                }
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
