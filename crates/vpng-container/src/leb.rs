//! Little-endian primitive readers.
//!
//! These functions index a slice directly. Callers must guarantee that the
//! underlying buffer is long enough, which is why they are crate-private and
//! only used after explicit length checks.

pub(crate) fn read_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

pub(crate) fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

pub(crate) fn read_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
        buf[offset + 4],
        buf[offset + 5],
        buf[offset + 6],
        buf[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn little_endian_reads() {
        let buf = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        assert_eq!(read_u16(&buf, 0), 0x0201);
        assert_eq!(read_u32(&buf, 0), 0x0403_0201);
        assert_eq!(read_u64(&buf, 1), 0x0908_0706_0504_0302);
    }
}
