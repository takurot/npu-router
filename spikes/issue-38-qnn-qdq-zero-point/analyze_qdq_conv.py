#!/usr/bin/env python3
"""Issue #38 investigation: decode docs/TEST_FIXTURES.md's pinned
`qdq-conv.onnx` fixture's ONNX protobuf by hand and compute the
mathematically correct (unsaturated -> saturated) output, to determine
whether the CPU golden (all 255) or the QNN HTP result observed in
spikes/p0-04-qnn-only-session/ (all 0) is the value the model's own
declared quantization parameters actually produce.

No `onnx`/`protobuf` package required or available in this environment, so
this is a minimal, purpose-built decoder for exactly the subset of the ONNX
wire format this one fixture uses (ModelProto -> GraphProto -> NodeProto /
TensorProto), not a general ONNX/protobuf library. See ../p0-03-ort-cpu-session
and ../p0-04-qnn-only-session for the equivalent narrow-purpose-parser
convention used by the sibling Rust spikes.

Usage:
    python3 analyze_qdq_conv.py [path/to/qdq-conv.onnx]

Defaults to the repo's pinned fixture path if no argument is given.
"""

from __future__ import annotations

import hashlib
import struct
import sys
from pathlib import Path

# SHA-256 pinned by fixtures/catalog.json (id "qnn-static-uint8-qdq-conv").
EXPECTED_SHA256 = "398abf0cea762e52116956e55358fe74d73fd2497a577071e48dc7e41cf79c50"

DTYPE_NAMES = {1: "FLOAT", 2: "UINT8", 3: "INT8", 6: "INT32", 7: "INT64"}


def read_varint(buf: bytes, pos: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while True:
        b = buf[pos]
        pos += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            return result, pos
        shift += 7


def parse_fields(buf: bytes) -> list[tuple[int, str, object]]:
    """Parses one protobuf message into a flat list of (field, wire_kind, value)."""
    pos = 0
    out: list[tuple[int, str, object]] = []
    while pos < len(buf):
        tag, pos = read_varint(buf, pos)
        field, wire = tag >> 3, tag & 7
        if wire == 0:
            val, pos = read_varint(buf, pos)
            out.append((field, "varint", val))
        elif wire == 2:
            length, pos = read_varint(buf, pos)
            out.append((field, "len", buf[pos : pos + length]))
            pos += length
        elif wire == 5:
            out.append((field, "i32", buf[pos : pos + 4]))
            pos += 4
        elif wire == 1:
            out.append((field, "i64", buf[pos : pos + 8]))
            pos += 8
        else:
            raise ValueError(f"unsupported wire type {wire} at byte {pos}")
    return out


def packed_varints(buf: bytes) -> list[int]:
    pos, out = 0, []
    while pos < len(buf):
        v, pos = read_varint(buf, pos)
        out.append(v)
    return out


def to_signed32(v: int) -> int:
    v &= 0xFFFFFFFF
    return v - 0x1_0000_0000 if v >= 0x8000_0000 else v


def parse_tensor_proto(buf: bytes) -> dict:
    """TensorProto subset: dims(1), data_type(2), int32_data(5, packed
    varint -- per the ONNX spec this field also carries uint8/int8/bool
    element values, one varint each, NOT 4-byte ints), float_data(4, packed
    fixed32), name(8), raw_data(9)."""
    dims: list[int] = []
    dtype = None
    name = None
    raw = None
    int32s = None
    floats = None
    for field, kind, value in parse_fields(buf):
        if field == 1 and kind == "varint":
            dims.append(value)
        elif field == 2 and kind == "varint":
            dtype = value
        elif field == 4 and kind == "len":
            floats = struct.unpack(f"<{len(value) // 4}f", value)
        elif field == 5 and kind == "len":
            int32s = packed_varints(value)
        elif field == 8 and kind == "len":
            name = value.decode()
        elif field == 9 and kind == "len":
            raw = value
    return dict(dims=dims, dtype=dtype, name=name, raw=raw, int32s=int32s, floats=floats)


def load_initializers(model_path: Path) -> dict[str, dict]:
    data = model_path.read_bytes()
    actual_sha256 = hashlib.sha256(data).hexdigest()
    if actual_sha256 != EXPECTED_SHA256:
        raise SystemExit(
            f"refusing to analyze: {model_path} sha256={actual_sha256} "
            f"does not match pinned fixtures/catalog.json value {EXPECTED_SHA256}"
        )

    model_fields = parse_fields(data)
    (graph_bytes,) = [v for f, k, v in model_fields if f == 7 and k == "len"]

    initializers: dict[str, dict] = {}
    for field, kind, value in parse_fields(graph_bytes):
        if field == 5 and kind == "len":  # GraphProto.initializer
            tensor = parse_tensor_proto(value)
            initializers[tensor["name"]] = tensor
    return initializers


def scalar(tensor: dict) -> int | float:
    dtype = DTYPE_NAMES.get(tensor["dtype"], tensor["dtype"])
    if dtype == "FLOAT":
        return tensor["floats"][0]
    if dtype == "INT32":
        return to_signed32(tensor["int32s"][0])
    if dtype in ("UINT8", "INT8"):
        return tensor["int32s"][0]
    raise ValueError(f"unhandled scalar dtype {dtype}")


def weight_grid(tensor: dict) -> list[int]:
    assert DTYPE_NAMES.get(tensor["dtype"]) == "UINT8"
    return list(tensor["int32s"])


def main() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    default_path = repo_root / ".cache" / "test-fixtures" / "qnn" / "qdq-conv.onnx"
    model_path = Path(sys.argv[1]) if len(sys.argv) > 1 else default_path
    if not model_path.exists():
        raise SystemExit(
            f"{model_path} not found -- run `node scripts/fetch-test-fixtures.mjs` first"
        )

    init = load_initializers(model_path)

    scale = scalar(init["Scale_0"])
    zp_uint8 = scalar(init["Zero_point_uint8_0"])
    zp_int32 = scalar(init["Zero_point_int32_0"])
    weights = weight_grid(init["W_0"])  # 3x3, row-major
    bias = scalar(init["Bias_0"])

    print("Decoded QDQ parameters (shared by every DequantizeLinear/QuantizeLinear node --")
    print("this fixture reuses one Scale_0/Zero_point_uint8_0 pair for input, weight, and")
    print("output, per the graph's own node list):")
    print(f"  Scale_0            = {scale}")
    print(f"  Zero_point_uint8_0 = {zp_uint8}")
    print(f"  Zero_point_int32_0 = {zp_int32}")
    print(f"  W_0 (3x3, uint8)   = {weights}")
    print(f"  Bias_0 (int32)     = {bias}")
    print()

    # Pinned golden input (fixtures/catalog.json "qnn-static-uint8-qdq-conv-input"):
    # sequential bytes 0..24, row-major [1,1,5,5].
    input_grid = list(range(25))

    def px(y: int, x: int) -> int:
        return input_grid[y * 5 + x]

    w_dq = [(w - zp_uint8) * scale for w in weights]  # constant 128*256 for every tap here
    bias_dq = (bias - zp_int32) * scale

    print("Per-output-position float conv accumulator (dequantized domain), then the")
    print("QuantizeLinear result (round-to-nearest, clamp to [0,255]):")
    all_saturate_high = True
    for oy in range(3):
        row_out = []
        for ox in range(3):
            acc = bias_dq
            for ky in range(3):
                for kx in range(3):
                    x_dq = (px(oy + ky, ox + kx) - zp_uint8) * scale
                    acc += x_dq * w_dq[ky * 3 + kx]
            requantized = acc / scale + zp_uint8
            quantized = max(0, min(255, round(requantized)))
            row_out.append(quantized)
            if quantized != 255:
                all_saturate_high = False
        print(f"  row {oy}: {row_out}")

    print()
    if all_saturate_high:
        print(
            "=> Every output position saturates to the maximum representable uint8 (255) "
            "*regardless of position*, because Scale_0=256 and the weight/bias magnitudes "
            "here make the pre-clamp accumulator many orders of magnitude larger than 255 "
            "for every 3x3 window. This is the CPU golden's answer -- it is the ONLY value "
            "consistent with this model's own quantization parameters."
        )
        print(
            "=> The QNN HTP result observed on real hardware "
            "(spikes/p0-04-qnn-only-session/, all-0) is therefore not an alternate-but-valid "
            "quantization interpretation -- it is arithmetically inconsistent with the "
            "model's own Scale_0/Zero_point_uint8_0/Zero_point_int32_0/W_0/Bias_0 values."
        )
    else:
        print("=> Not every position saturates -- re-check this script against the model.")


if __name__ == "__main__":
    main()
