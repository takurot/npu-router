# Phase 0 test fixture

Issue #3、`docs/SPEC.md` 22.4、OQ-04 に対応する CPU/QNN spike 共通 fixture をここで固定する。

## 選定

| 用途 | artifact | shape / dtype | golden / 許容誤差 | 選定理由 |
| --- | --- | --- | --- | --- |
| tiny classification | ONNX Model Zoo `mnist-12.tar.gz` | input `[1,1,28,28]` `float32`、output `[1,10]` `float32` | archive 内 `test_data_set_0`。`atol=1e-4`、`rtol=1e-4` | 26 KiB で model と TensorProto の既知入出力が揃う |
| QNN 対応候補 | ORT v1.23.2 `qdq_conv.onnx` | input `[1,1,5,5]` `uint8`、output `[1,1,3,3]` `uint8` | `qnn/golden/*.raw`。CPU golden は全要素 `255`、CPU/QNN 差は最大 1 LSB | 静的 shape、uint8 QDQ、QNN の対応表にある `Conv` / `QuantizeLinear` / `DequantizeLinear` だけで構成される |
| QNN 非対応 | ORT v1.23.2 `qnn_ep_partial_support.onnx` | inputs `x,y:[2,2] int8`、`z:[2,2] float32`、output `[2,2] float32` | fallback 経路の判定用。数値 golden の対象外 | ORT 自身の QNN test が `MatMulInteger` を非対応として CPU fallback 無効時の Session 生成失敗に使用する |

QNN の基準量子化形式（OQ-04）は、MVP の技術 spike では「静的 shape の uint8 QDQ」とする。`qdq_conv.onnx` は QNN 対応を示す候補であり、QNN HTP で model 全体が割り当てられることの確証は Issue #5 の Windows ARM64 実機ログと `session.disable_cpu_ep_fallback=1` で取得する。実機未検証の状態を QNN 対応済みとは扱わない。

MNIST model card は model license を MIT と記載する。ONNX Model Zoo repository は Apache-2.0、ONNX Runtime の 2 artifact は MIT である。取得元は mutable branch ではなく次の commit に固定した。

- ONNX Model Zoo: `c32b9776d06d2ebc7888d705e3a558f62b20e7a8`
- ONNX Runtime v1.23.2 tag: `a83fc4d58cb48eb68890dd689f94f28288cf2278`

URL、byte size、SHA-256 は [`fixtures/catalog.json`](../fixtures/catalog.json) が正本である。license と model card はそれぞれの固定 commit の `LICENSE` および MNIST `README.md` で確認する。

## 取得と検証

Node.js 22 以上で実行する。

```bash
node scripts/fetch-test-fixtures.mjs
node scripts/fetch-test-fixtures.mjs --verify-only
```

artifact は `.cache/test-fixtures/` に保存され、Git には追加しない。取得処理は HTTPS の許可 host、宣言 size、SHA-256、保存先 containment を検証し、検証後に atomic rename する。既存 artifact は毎回 SHA-256 を再検証する。

MNIST archive の内訳も固定する。

| member | bytes | SHA-256 |
| --- | ---: | --- |
| `mnist-12/mnist-12.onnx` | 26143 | `5c688690f8bacf667d4c2074af5ad0646ca328d7ab03eccf944a65b320171bdd` |
| `mnist-12/test_data_set_0/input_0.pb` | 3157 | `d44b08082c3ded89e081f699a9d604239818c805ee8b5d03cd80f338e641c720` |
| `mnist-12/test_data_set_0/output_0.pb` | 66 | `153a5b1d96f9a544fc398f8c1837b994bbf5f26d3d12a7eff2ec63f7fb2317e1` |

archive の展開先は一時 directory とし、上表以外の member、絶対 path、`..`、symlink を拒否する。展開処理は ORT integration test の実装時にこの契約で追加する。

## malformed corpus 方針

malformed corpus は外部 binary を追加せず、各 consumer の test 内で決定的に生成する。

- manifest: 正常 fixture から shape、dtype、checksum、相対 path を一項目ずつ変更する。path traversal と symlink escape は一時 model root 内で構成する。
- image: header truncation、decode 後 size 上限超過、channel 不一致を最小 byte 列から生成する。実画像や個人データを格納しない。
- model: 検証済み model の copy を一時 directory で truncate または 1 byte 変更する。破損 artifact を catalog へ登録しない。

各 test は期待 error code、Provider 呼出回数 0、作成される一時 file の上限を検証する。fuzzing が見つけた regression input は最小化し、license と機密性を確認した小さい fixture だけを `fixtures/regressions/` に追加する。

## 保存方式の決定

- repository: catalog、取得/検証コード、数十 byte の inline golden、malformed corpus 方針を保存する。
- Git LFS: 使用しない。現状の artifact 合計が小さく、上流 commit と SHA-256 から再取得できるためである。
- release artifact: clean-machine / offline qualification が必要になる Phase 4 までは作成しない。配布する場合も同じ catalog と SHA-256 を provenance metadata に含める。

## 実機で残る検証

- Windows 11 ARM64 / Snapdragon X Plus または Elite / QNN HTP で `qdq_conv.onnx` が CPU fallback 無効の Session として生成・実行できること
- QNN output が CPU golden から最大 1 LSB 以内であること
- `qnn_ep_partial_support.onnx` が QNN-only Session 生成に失敗し、明示的 fallback test では CPU で成功すること

これらは Issue #5 の成果物であり、本 Issue の host-side mock や CPU 実行で完了扱いにしない。
