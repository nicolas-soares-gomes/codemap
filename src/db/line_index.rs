//! Per-line byte offsets (varint delta-encoded), so a symbol range can be read straight
//! from disk without storing code or rescanning the whole file.

/// Byte offset of the start of each line. `offsets[0] == 0`; len == line count.
pub fn compute_offsets(bytes: &[u8]) -> Vec<u64> {
    let mut offs = Vec::with_capacity(64);
    offs.push(0u64);
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            offs.push((i + 1) as u64);
        }
    }
    offs
}

pub fn encode(offsets: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(offsets.len() * 2);
    let mut prev = 0u64;
    for &o in offsets {
        write_varint(&mut out, o - prev);
        prev = o;
    }
    out
}

pub fn decode(blob: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut prev = 0u64;
    let mut i = 0;
    while i < blob.len() {
        let (delta, n) = read_varint(&blob[i..]);
        prev += delta;
        out.push(prev);
        i += n;
    }
    out
}

/// Byte span `[start, end)` covering 1-based lines `[start_line..=end_line]`.
pub fn byte_span(offsets: &[u64], total_len: u64, start_line: u32, end_line: u32) -> (u64, u64) {
    let start = offsets
        .get(start_line.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(0);
    let end = offsets.get(end_line as usize).copied().unwrap_or(total_len);
    (start.min(total_len), end.min(total_len))
}

fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

fn read_varint(buf: &[u8]) -> (u64, usize) {
    let mut v = 0u64;
    let mut shift = 0;
    let mut n = 0;
    for &b in buf {
        v |= ((b & 0x7f) as u64) << shift;
        n += 1;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    (v, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_offsets() {
        let src = b"line1\nline2\n\nend";
        let offs = compute_offsets(src);
        assert_eq!(offs, vec![0, 6, 12, 13]);
        assert_eq!(decode(&encode(&offs)), offs);
    }

    #[test]
    fn byte_span_extracts_exact_lines() {
        let src = b"aaa\nbbb\nccc\nddd";
        let offs = compute_offsets(src);
        let (b0, b1) = byte_span(&offs, src.len() as u64, 2, 3);
        assert_eq!(&src[b0 as usize..b1 as usize], b"bbb\nccc\n");
        let (c0, c1) = byte_span(&offs, src.len() as u64, 4, 4);
        assert_eq!(&src[c0 as usize..c1 as usize], b"ddd");
    }
}
