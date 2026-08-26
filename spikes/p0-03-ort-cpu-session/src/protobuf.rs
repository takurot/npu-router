//! Minimal, fixture-specific TensorProto reader.
//!
//! Not a general protobuf parser (see crate root "Non-goals"): it
//! understands exactly the three fields the mnist-12 fixtures use
//! (1=dims varint repeated, 2=data_type varint, 9=raw_data bytes) and skips
//! any other field by wire type so it tolerates the `name` (field 8) and
//! similar metadata fields without needing to interpret them. All reads are
//! bounds-checked; malformed input yields `SpikeError::Protobuf`, never a
//! panic or out-of-bounds read.

use crate::error::SpikeError;

#[derive(Debug, Default)]
pub struct ParsedTensor {
    pub dims: Vec<i64>,
    pub data_type: i32,
    pub raw_data: Vec<u8>,
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, SpikeError> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *buf
            .get(*pos)
            .ok_or_else(|| SpikeError::Protobuf("truncated varint".into()))?;
        *pos += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err(SpikeError::Protobuf("varint too long".into()));
        }
    }
}

fn take<'a>(buf: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], SpikeError> {
    let end = pos
        .checked_add(len)
        .ok_or_else(|| SpikeError::Protobuf("length overflow".into()))?;
    let slice = buf
        .get(*pos..end)
        .ok_or_else(|| SpikeError::Protobuf("truncated field".into()))?;
    *pos = end;
    Ok(slice)
}

pub fn parse_tensor_proto(buf: &[u8]) -> Result<ParsedTensor, SpikeError> {
    let mut pos = 0usize;
    let mut out = ParsedTensor::default();
    while pos < buf.len() {
        let tag = read_varint(buf, &mut pos)?;
        let field = tag >> 3;
        let wire = tag & 0x7;
        match (field, wire) {
            (1, 0) => out.dims.push(read_varint(buf, &mut pos)? as i64),
            (2, 0) => out.data_type = read_varint(buf, &mut pos)? as i32,
            (9, 2) => {
                let len = read_varint(buf, &mut pos)? as usize;
                out.raw_data = take(buf, &mut pos, len)?.to_vec();
            }
            (_, 0) => {
                read_varint(buf, &mut pos)?;
            }
            (_, 2) => {
                let len = read_varint(buf, &mut pos)? as usize;
                take(buf, &mut pos, len)?;
            }
            (_, 1) => {
                take(buf, &mut pos, 8)?;
            }
            (_, 5) => {
                take(buf, &mut pos, 4)?;
            }
            _ => {
                return Err(SpikeError::Protobuf(format!(
                    "unsupported wire type {wire}"
                )));
            }
        }
    }
    Ok(out)
}

pub fn f32_vec_from_le_bytes(bytes: &[u8]) -> Result<Vec<f32>, SpikeError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(SpikeError::Protobuf(
            "raw_data length not a multiple of 4".into(),
        ));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real, unmodified bytes of the mnist-12 fixture's
    /// `test_data_set_0/output_0.pb` (SHA-256
    /// `153a5b1d96f9a544fc398f8c1837b994bbf5f26d3d12a7eff2ec63f7fb2317e1`,
    /// pinned by `docs/TEST_FIXTURES.md`): `TensorProto{dims=[1,10],
    /// data_type=FLOAT, name="Plus214_Output_0", raw_data=<40 bytes>}`.
    #[rustfmt::skip]
    const OUTPUT_0_PB: &[u8] = &[
        0x08, 0x01, 0x08, 0x0a, 0x10, 0x01, 0x42, 0x10, 0x50, 0x6c, 0x75, 0x73, 0x32, 0x31, 0x34,
        0x5f, 0x4f, 0x75, 0x74, 0x70, 0x75, 0x74, 0x5f, 0x30, 0x4a, 0x28, 0x0a, 0x2f, 0x14, 0xc2,
        0x95, 0x05, 0xb0, 0xc0, 0x06, 0x14, 0xc3, 0x41, 0x2a, 0xf3, 0x7a, 0x42, 0x22, 0x14, 0x0e,
        0x42, 0xca, 0x77, 0x07, 0x42, 0x00, 0xb6, 0xe8, 0xc1, 0x4b, 0x0c, 0x18, 0x42, 0x6a, 0x4a,
        0x34, 0xc2, 0x1d, 0x30, 0x8f, 0xc2,
    ];

    #[test]
    fn parses_real_mnist_golden_output_tensor_proto() {
        let parsed = parse_tensor_proto(OUTPUT_0_PB).expect("valid fixture bytes must parse");

        assert_eq!(parsed.dims, vec![1, 10]);
        assert_eq!(parsed.data_type, 1); // ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT
        assert_eq!(parsed.raw_data.len(), 40); // 10 x float32

        let floats = f32_vec_from_le_bytes(&parsed.raw_data).unwrap();
        assert_eq!(floats.len(), 10);
    }

    #[test]
    fn rejects_truncated_varint() {
        let err = parse_tensor_proto(&[0x80]).unwrap_err();
        assert!(matches!(err, SpikeError::Protobuf(_)));
    }

    #[test]
    fn rejects_length_delimited_field_that_overruns_the_buffer() {
        // field 9 (raw_data), wire type 2, declared length 100 but only 1
        // byte follows: must error, not read out of bounds.
        let buf = [0x4a, 0x64, 0x00];
        let err = parse_tensor_proto(&buf).unwrap_err();
        assert!(matches!(err, SpikeError::Protobuf(_)));
    }

    #[test]
    fn skips_unknown_fields_by_wire_type() {
        // field 20 (unknown), wire type 0 (varint) = 42, followed by the
        // real fields 1 (dims=[5]) and 2 (data_type=1).
        let buf = [0xa0, 0x01, 0x2a, 0x08, 0x05, 0x10, 0x01];
        let parsed = parse_tensor_proto(&buf).unwrap();
        assert_eq!(parsed.dims, vec![5]);
        assert_eq!(parsed.data_type, 1);
    }

    #[test]
    fn f32_conversion_rejects_length_not_multiple_of_four() {
        let err = f32_vec_from_le_bytes(&[0, 0, 0]).unwrap_err();
        assert!(matches!(err, SpikeError::Protobuf(_)));
    }
}
