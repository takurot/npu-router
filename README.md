# NPU Router

Windows 11 ARM64 PCのNPUを、Rust SDKまたはローカルHTTP APIから利用するための推論ミドルウェアです。

NPU RouterはONNX RuntimeのExecution Provider差異をアプリケーションから隠蔽し、Qualcomm QNN NPUを優先して推論します。QNNの初期化や実行に失敗した場合は、モデルポリシーに従って独立したCPU Sessionへfallbackします。

> [!NOTE]
> 現在は設計・技術検証段階です。実行可能なRuntimeはまだ実装されていません。

## 目標

アプリケーション開発者が、QNN SDKやONNX RuntimeのProvider設定を直接扱わずにローカルAI推論を実行できる状態を目指します。

```rust
let router = NpuRouter::builder()
    .model_dir("./models")
    .build()?;

let result = router
    .model("person-detector")
    .infer(input)?;
```

HTTP APIからも同じRuntimeを利用できます。

```http
POST /v1/infer/person-detector
```

## MVP

- Windows 11 ARM64
- Snapdragon X Plus / Snapdragon X Elite
- ONNX Runtime
- QNN Execution Provider
- CPU Execution Providerへの明示的fallback
- 静的shapeのONNXモデル
- Rust SDK
- localhost限定HTTP API
- 環境診断、モデル管理、推論、ベンチマークCLI
- 構造化ログと基本メトリクス

DirectML、Windows ML、AMD/Intel NPU、動的shape、モデル自動量子化、LLM生成はMVPの対象外です。

## アーキテクチャ

```text
Rust Application       CLI                Local HTTP Client
       |                |                         |
       +----------------+-------------------------+
                        |
                 NpuRouter Core API
                        |
       +----------------+----------------+
       |                |                |
 Model Registry   Inference Router   Diagnostics
                        |
             +----------+----------+
             |                     |
      QNN-only Session       CPU-only Session
             |                     |
             +----------+----------+
                        |
              ONNX Runtime C API
```

QNN Session内の暗黙CPU fallbackは使用しません。QNNとCPUのSessionを分離することで、レスポンスに含まれる`provider`と`fallback_used`が実際の実行経路を正確に表す設計です。

詳細は[機能仕様書](docs/SPEC.md)を参照してください。

## 想定する構成

```text
npu-router/
  crates/
    npu-core/       # ドメイン型、Registry、Router、Session lifecycle
    npu-ort/        # ONNX Runtime C API連携
    npu-qnn/        # QNN EP設定と診断
    npu-tasks/      # 前処理・後処理
    npu-sdk/        # 公開Rust API
    npu-cli/        # CLI
    npu-server/     # HTTP Server
  docs/
    IDEA.md
    SPEC.md
```

## CLI計画

```text
npu doctor [--json] [--run-smoke-test]
npu providers [--json]
npu models [--json]
npu model register <manifest-path>
npu model validate <manifest-path> [--provider qnn|cpu]
npu model load <name[@version]>
npu model unload <name[@version]>
npu infer <name[@version]> --input <path> [--provider auto|qnn|cpu]
npu benchmark <name[@version]> --input <path> --provider <qnn|cpu>
npu serve [--bind 127.0.0.1:8080]
```

## 開発ロードマップ

1. 技術検証
   - ONNX Runtime、QNN SDK、QNN backendの互換バージョンを固定
   - Windows ARM64実機でCPU/QNN Sessionを生成
   - QNN内のCPU fallback無効化を検証
2. Core + CPU
   - Model Registry、Session Manager、Rust SDK
   - CPU推論と画像分類
3. QNN + fallback
   - QNN診断、QNN専用Session、CPU fallback
4. CLI + HTTP
   - ローカルAPI、負荷制御、ログ、メトリクス
5. Vision PoC
   - YOLO系モデルによる物体検出とベンチマーク

## ドキュメント

- [IDEA.md](docs/IDEA.md) — 背景、プロダクト構想、ユースケース
- [SPEC.md](docs/SPEC.md) — アーキテクチャ、機能契約、受け入れ基準

## 実装前の主要な未決事項

- ONNX Runtime、QNN SDK、QNN backendの対応バージョン
- Runtime DLLの配布方法とライセンス条件
- Rust bindingを利用するか、最小C API wrapperを実装するか
- QNN基準モデルと量子化形式
- ONNX Runtime実行のtimeout/cancel挙動

これらはPhase 0の技術検証とArchitecture Decision Recordで確定します。

## セキュリティ方針

- HTTP Serverはlocalhostにのみbind
- モデルルート外のファイル参照を拒否
- 入力サイズ、Tensorサイズ、queue長を制限
- DLL探索順序を固定
- 入力画像やTensor値を標準ログへ記録しない

## ステータス

Architecture Review Ready。次の作業はPhase 0の技術spikeとADR作成です。
