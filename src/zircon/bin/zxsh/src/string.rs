// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use bstr::{BStr, BString, ByteSlice};
use std::ffi::{CStr, CString, NulError};

/// Parse an integer from a byte slice, ignoring leading and trailing ASCII whitespace.
/// Supports optional leading '+' or '-'.
pub fn parse_int<T: std::str::FromStr>(bytes: &[u8]) -> Option<T> {
    let trimmed = bytes.trim_ascii();
    if trimmed.is_empty() {
        return None;
    }
    std::str::from_utf8(trimmed).ok()?.parse::<T>().ok()
}

/// Parse a file mode mask (in octal or symbolic format) from a byte slice.
/// `current_umask` is used as the baseline when evaluating symbolic mode modifications (+, -, =).
/// Returns `Some(new_umask)` if the mode string is valid, or `None` if invalid.
pub fn parse_mode_mask(bytes: &[u8], current_umask: u32) -> Option<u32> {
    let trimmed = bytes.trim_ascii();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.iter().all(|c| c.is_ascii_digit()) {
        let s = std::str::from_utf8(trimmed).ok()?;
        let val = u32::from_str_radix(s, 8).ok()?;
        if val > 0o777 {
            return None;
        }
        return Some(val);
    }

    let str_val = std::str::from_utf8(trimmed).ok()?;
    let mut perm = 0o777 & !current_umask;
    for clause in str_val.split(',') {
        let mut char_indices = clause.char_indices();
        let mut op_char = None;
        let mut op_byte_idx = None;
        while let Some((idx, ch)) = char_indices.next() {
            if ch == '+' || ch == '-' || ch == '=' {
                op_char = Some(ch);
                op_byte_idx = Some(idx);
                break;
            }
        }
        let (op, op_idx) = match (op_char, op_byte_idx) {
            (Some(op_ch), Some(idx)) => (op_ch, idx),
            _ => return None,
        };
        let who_str = &clause[..op_idx];
        let perm_str = &clause[op_idx + op.len_utf8()..];

        let who_mask = if who_str.is_empty() {
            0o777
        } else {
            let mut who_bits = 0;
            for ch in who_str.chars() {
                match ch {
                    'u' => who_bits |= 0o700,
                    'g' => who_bits |= 0o070,
                    'o' => who_bits |= 0o007,
                    'a' => who_bits |= 0o777,
                    _ => return None,
                }
            }
            who_bits
        };

        let mut p_bits = 0;
        for ch in perm_str.chars() {
            match ch {
                'r' => p_bits |= (0o400 | 0o040 | 0o004) & who_mask,
                'w' => p_bits |= (0o200 | 0o020 | 0o002) & who_mask,
                'x' => p_bits |= (0o100 | 0o010 | 0o001) & who_mask,
                _ => return None,
            }
        }

        match op {
            '+' => perm |= p_bits,
            '-' => perm &= !p_bits,
            '=' => {
                perm &= !who_mask;
                perm |= p_bits;
            }
            _ => unreachable!(),
        }
    }

    Some(0o777 & !perm)
}

/// Converts a byte slice (e.g., `&BStr` or `&BString`) into a `CString` for FFI calls.
pub fn bstr_to_cstring(s: &[u8]) -> Result<CString, NulError> {
    CString::new(s)
}

/// Converts a slice of `BString`s into a vector of `CString`s for FFI calls.
pub fn bstrings_to_cstrings(strings: &[BString]) -> Result<Vec<CString>, NulError> {
    strings.iter().map(|s| bstr_to_cstring(s.as_slice())).collect()
}

/// Converts a slice of `CString`s into a vector of `&CStr` references for FFI calls.
pub fn cstrings_to_c_strs(cstrings: &[CString]) -> Vec<&CStr> {
    cstrings.iter().map(|s| s.as_c_str()).collect()
}

/// Splits a byte slice around the first `=` character into `(key, value)`.
/// Returns `Some((&BStr, &BStr))` if `=` is present, or `None` if there is no `=`.
pub fn split_key_value(bytes: &[u8]) -> Option<(&BStr, &BStr)> {
    bytes.find_byte(b'=').map(|idx| (BStr::new(&bytes[..idx]), BStr::new(&bytes[idx + 1..])))
}

/// Splits input bytes according to IFS rules for the shell `read` command into `num_vars` fields.
pub fn split_ifs_read(input: &[u8], ifs: &BStr, num_vars: usize) -> Vec<BString> {
    if num_vars == 0 {
        return Vec::new();
    }

    let is_ifs_whitespace =
        |c: u8| (c == b' ' || c == b'\t' || c == b'\n') && ifs.as_bytes().contains(&c);
    let is_ifs_non_whitespace = |c: u8| !is_ifs_whitespace(c) && ifs.as_bytes().contains(&c);

    let mut fields = Vec::new();
    let mut start = 0;

    // Skip leading IFS whitespace
    while start < input.len() && is_ifs_whitespace(input[start]) {
        start += 1;
    }

    for _ in 0..num_vars - 1 {
        if start >= input.len() {
            break;
        }

        let mut i = start;
        let mut delim_start = None;
        let mut delim_end = None;

        while i < input.len() {
            let c = input[i];
            if is_ifs_whitespace(c) {
                if delim_start.is_none() {
                    delim_start = Some(i);
                }
                i += 1;
                while i < input.len() && is_ifs_whitespace(input[i]) {
                    i += 1;
                }
                if i < input.len() && is_ifs_non_whitespace(input[i]) {
                    i += 1;
                    while i < input.len() && is_ifs_whitespace(input[i]) {
                        i += 1;
                    }
                }
                delim_end = Some(i);
                break;
            } else if is_ifs_non_whitespace(c) {
                delim_start = Some(i);
                i += 1;
                while i < input.len() && is_ifs_whitespace(input[i]) {
                    i += 1;
                }
                delim_end = Some(i);
                break;
            } else {
                i += 1;
            }
        }

        if let (Some(ds), Some(de)) = (delim_start, delim_end) {
            let field = BString::from(&input[start..ds]);
            fields.push(field);
            start = de;
        } else {
            let field = BString::from(&input[start..]);
            fields.push(field);
            start = input.len();
        }
    }

    if start < input.len() {
        let mut end = input.len();
        while end > start && is_ifs_whitespace(input[end - 1]) {
            end -= 1;
        }
        let mut last_field = input[start..end].to_vec();
        let ifs_non_whitespace_count =
            last_field.iter().filter(|&&c| is_ifs_non_whitespace(c)).count();
        if ifs_non_whitespace_count == 1
            && last_field.last().map_or(false, |&c| is_ifs_non_whitespace(c))
        {
            last_field.pop();
        }
        fields.push(BString::from(last_field));
    }

    while fields.len() < num_vars {
        fields.push(BString::default());
    }

    fields
}
