# NPU Router 機能仕様書 v0.2

## 1. 概要

### 1.1 プロダクト名

**NPU Router**

仮称。

### 1.2 目的

NPU Routerは、Windows ARM PC上のNPUをアプリケーションから簡単に利用するためのミドルウェアである。

開発者は、QNN SDK、ONNX Runtime、Execution Provider、DirectML、CPU fallbackなどの差異を意識せず、単一のAPIでAI推論を実行できる。

### 1.3 目指す状態

アプリケーション開発者が以下のようなコードだけでNPU推論を利用できる状態を目指す。

```rust
let runtime = NpuRouter::new()?;

let result = runtime
    .model("person-detector")
    .infer(input)?;
```

またはHTTP APIで以下のように利用できる。

```http
POST /v1/infer/person-detector
```

NPU Routerは内部で、利用可能なExecution Providerを確認し、最適な実行先を選択する。

```text
QNN NPU
  ↓ 失敗時
DirectML / GPU
  ↓ 失敗時
CPU
```

---

## 2. 背景と課題

### 2.1 背景

Snapdragon Xシリーズを搭載したWindows ARM PCでは、NPUを利用したローカルAI推論が可能になっている。

一方で、実際にアプリケーションからNPUを使うには、以下の知識が必要になる。

* ONNX Runtime
* QNN Execution Provider
* Qualcomm QNN SDK
* Windows ARM64環境
* モデル変換
* 量子化
* Unsupported Operator対応
* CPU fallback
* DirectML
* Windows ML

これらはアプリケーション開発者にとって負担が大きい。

### 2.2 解決する課題

NPU Routerは以下の課題を解決する。

| 課題              | 解決方針               |
| --------------- | ------------------ |
| NPUが使えるか分からない   | `npu doctor`で診断    |
| QNN EP設定が難しい    | ミドルウェア側で抽象化        |
| モデルがNPU対応か分からない | モデル検証機能で判定         |
| 実行時に失敗する        | CPU / GPU fallback |
| アプリごとに実装が重複する   | 共通Runtimeとして提供     |
| NPU性能が見えない      | ベンチマークとメトリクス提供     |
| Rustから使いにくい     | Rust SDKを第一級対応     |

---

## 3. 対象ユーザー

### 3.1 Primary User

* Windows ARM PCでAIアプリを作る開発者
* Rust / Python / C++ 開発者
* エッジAI開発者
* ロボティクス開発者
* 工場DX / 物流DX向けAI開発者

### 3.2 Secondary User

* AIモデルを配布する開発者
* ローカルAIアプリを作るSaaS開発者
* PoC環境でNPU活用を試したいR&D担当者

---

## 4. 対応環境

### 4.1 MVP対象環境

| 項目           | 内容                                     |
| ------------ | -------------------------------------- |
| OS           | Windows 11 ARM64                       |
| CPU          | Snapdragon X Plus / Snapdragon X Elite |
| NPU          | Qualcomm Hexagon NPU                   |
| Runtime      | ONNX Runtime                           |
| NPU Provider | QNN Execution Provider                 |
| 開発言語         | Rust                                   |
| 外部API        | HTTP                                   |

### 4.2 初期検証対象PC

```text
Processor:
Snapdragon X 10-core X1P64100

Architecture:
ARM64

RAM:
16GB

OS:
Windows 11 ARM64
```

### 4.3 将来対応候補

* AMD Ryzen AI
* Intel Core Ultra NPU
* Windows ML
* Linux ARM64
* Android
* Jetson
* WebGPU / WebNN

---

## 5. スコープ

### 5.1 MVPで対応するもの

MVPでは以下に対応する。

* ONNXモデルのロード
* QNN Execution Providerによる推論
* CPU fallback
* モデル登録
* モデル設定ファイル
* CLI診断
* CLIベンチマーク
* Rust SDK
* HTTP API
* 推論ログ
* 基本メトリクス

### 5.2 MVPで対応しないもの

初期MVPでは以下は対象外とする。

* LLM本体の高速生成
* KV Cache最適化
* 動的shapeの完全対応
* モデル自動量子化
* Windows ML統合
* GPU DirectML最適化
* 分散推論
* マルチノード管理
* モデルマーケットプレイス
* 認証・認可の本格実装

---

## 6. 想定ユースケース

### 6.1 画像分類

画像を入力し、NPUで分類結果を返す。

例：

```text
Input:
image.jpg

Output:
{
  "class": "forklift",
  "score": 0.94,
  "provider": "qnn"
}
```

### 6.2 物体検出

YOLO系の軽量モデルをNPUで実行し、検出結果を返す。

例：

```json
{
  "objects": [
    {
      "label": "person",
      "score": 0.91,
      "bbox": [120, 80, 220, 310]
    }
  ],
  "provider": "qnn",
  "latency_ms": 22
}
```

### 6.3 Embedding生成

テキストまたは画像からEmbeddingを生成する。

用途：

* ローカルRAG
* 類似検索
* 異常検知
* セマンティックイベント生成

### 6.4 工場見守りAI

カメラ映像から人物、フォークリフト、AMR、パレットなどを検出する。

NPU Routerは低消費電力で常時推論を行う基盤として利用する。

### 6.5 AMR交通整理

複数カメラ映像から人、AMR、フォークリフトを検出し、衝突リスクや渋滞リスクを上位アプリケーションに渡す。

---

## 7. 全体アーキテクチャ

```text
Application
  ↓
Rust SDK / HTTP API
  ↓
NPU Router Core
  ↓
Model Registry
  ↓
Runtime Manager
  ↓
Provider Manager
  ↓
ONNX Runtime
  ↓
QNN EP / DirectML EP / CPU EP
  ↓
NPU / GPU / CPU
```

---

## 8. コンポーネント設計

### 8.1 NPU Router Core

NPU Router全体の中心コンポーネント。

責務：

* 推論要求の受付
* モデル選択
* Runtime呼び出し
* Provider選択
* エラー処理
* fallback制御
* メトリクス記録

### 8.2 Model Registry

モデルのメタデータを管理する。

責務：

* モデル名管理
* モデルパス管理
* バージョン管理
* 入出力shape管理
* 推奨Provider管理
* fallback設定管理

### 8.3 Runtime Manager

ONNX Runtime Sessionを管理する。

責務：

* セッション生成
* セッション破棄
* モデルロード
* モデルアンロード
* ウォームアップ
* 推論実行
* セッションキャッシュ

### 8.4 Provider Manager

利用可能なExecution Providerを管理する。

責務：

* QNN EP利用可否判定
* CPU EP利用可否判定
* DirectML EP利用可否判定
* Provider優先順位決定
* Provider初期化
* Provider fallback

### 8.5 Diagnostics Manager

環境診断を行う。

責務：

* OS確認
* CPU確認
* アーキテクチャ確認
* ONNX Runtime確認
* QNN EP確認
* QNN関連DLL確認
* サンプルモデル推論確認

### 8.6 Metrics Manager

推論性能とRuntime状態を記録する。

責務：

* latency測定
* throughput測定
* model load time測定
* memory usage取得
* provider別成功率記録
* fallback発生率記録

---

## 9. モデル設定仕様

### 9.1 モデル定義ファイル

モデルはYAMLで登録する。

```yaml
name: person-detector
version: 0.1.0
format: onnx
path: ./models/person-detector/model.onnx

task: object_detection

preferred_provider: qnn

fallback:
  - cpu

input:
  name: images
  dtype: float32
  shape: [1, 3, 640, 640]
  layout: nchw

output:
  name: output0
  dtype: float32

preprocess:
  resize:
    width: 640
    height: 640
  normalize:
    mean: [0.0, 0.0, 0.0]
    std: [255.0, 255.0, 255.0]

postprocess:
  type: yolo
  confidence_threshold: 0.25
  nms_threshold: 0.45
```

### 9.2 モデル状態

モデルは以下の状態を持つ。

| 状態         | 意味      |
| ---------- | ------- |
| REGISTERED | 登録済み    |
| LOADING    | ロード中    |
| LOADED     | ロード済み   |
| FAILED     | ロード失敗   |
| UNLOADED   | アンロード済み |
| DISABLED   | 無効化     |

### 9.3 モデルバージョン

同一モデル名に複数バージョンを登録できる。

例：

```text
person-detector:0.1.0
person-detector:0.2.0
```

未指定時はlatestを使用する。

---

## 10. Provider選択仕様

### 10.1 Provider優先順位

初期MVPでは以下の優先順位とする。

```text
1. QNN
2. CPU
```

将来は以下を追加する。

```text
1. QNN
2. DirectML
3. Windows ML
4. CPU
```

### 10.2 自動fallback条件

以下の場合、次のProviderへfallbackする。

* Provider初期化失敗
* モデルロード失敗
* Unsupported Operator
* Tensor shape不一致
* メモリ不足
* 推論タイムアウト
* Provider実行時エラー

### 10.3 fallbackポリシー

fallbackポリシーはモデルごとに設定できる。

```yaml
fallback_policy:
  enabled: true
  providers:
    - qnn
    - cpu
  fail_fast: false
```

### 10.4 fail fast

`fail_fast: true` の場合、preferred providerで失敗した時点でエラーを返す。

NPU専用検証やベンチマーク時に利用する。

---

## 11. API仕様

## 11.1 Rust SDK

### Runtime初期化

```rust
use npu_router::NpuRouter;

let router = NpuRouter::builder()
    .model_dir("./models")
    .enable_fallback(true)
    .build()?;
```

### モデルロード

```rust
router.load_model("person-detector")?;
```

### 推論

```rust
let result = router
    .model("person-detector")
    .infer(input)?;
```

### Provider指定

```rust
let result = router
    .model("person-detector")
    .provider("qnn")
    .infer(input)?;
```

### メトリクス取得

```rust
let metrics = router.metrics("person-detector")?;
```

---

## 11.2 HTTP API

### Health Check

```http
GET /health
```

レスポンス：

```json
{
  "status": "ok",
  "runtime": "onnxruntime",
  "architecture": "arm64"
}
```

### Provider一覧

```http
GET /v1/providers
```

レスポンス：

```json
{
  "providers": [
    {
      "name": "qnn",
      "available": true,
      "status": "ready"
    },
    {
      "name": "cpu",
      "available": true,
      "status": "ready"
    }
  ]
}
```

### モデル一覧

```http
GET /v1/models
```

レスポンス：

```json
{
  "models": [
    {
      "name": "person-detector",
      "version": "0.1.0",
      "status": "loaded",
      "preferred_provider": "qnn"
    }
  ]
}
```

### 推論

```http
POST /v1/infer/person-detector
```

リクエスト：

```json
{
  "input_type": "image",
  "input": "base64-encoded-image"
}
```

レスポンス：

```json
{
  "model": "person-detector",
  "version": "0.1.0",
  "provider": "qnn",
  "latency_ms": 22,
  "fallback_used": false,
  "result": {
    "objects": [
      {
        "label": "person",
        "score": 0.91,
        "bbox": [120, 80, 220, 310]
      }
    ]
  }
}
```

---

## 12. CLI仕様

### 12.1 環境診断

```bash
npu doctor
```

出力例：

```text
NPU Router Doctor

OS:
  Windows 11 ARM64

CPU:
  Snapdragon X Plus

Architecture:
  ARM64

ONNX Runtime:
  Available

QNN Execution Provider:
  Available

QNN DLL:
  Found

Status:
  Ready
```

### 12.2 Provider確認

```bash
npu providers
```

出力例：

```text
Provider   Available   Status
QNN        yes         ready
CPU        yes         ready
DirectML   no          not_configured
```

### 12.3 モデル登録

```bash
npu model register ./models/person-detector/model.yaml
```

### 12.4 モデル一覧

```bash
npu models
```

### 12.5 モデルロード

```bash
npu model load person-detector
```

### 12.6 推論実行

```bash
npu infer person-detector --input ./sample.jpg
```

### 12.7 ベンチマーク

```bash
npu benchmark person-detector --provider qnn
```

出力例：

```text
Model:
  person-detector

Provider:
  qnn

Runs:
  100

Average latency:
  22.4 ms

P95 latency:
  29.8 ms

Throughput:
  44.6 fps

Fallback:
  false
```

---

## 13. エラー仕様

### 13.1 エラーコード

| コード    | 内容                   |
| ------ | -------------------- |
| NPU001 | QNN EPが利用できない        |
| NPU002 | モデルロード失敗             |
| NPU003 | Unsupported Operator |
| NPU004 | 入力shape不一致           |
| NPU005 | 推論タイムアウト             |
| NPU006 | fallback失敗           |
| NPU007 | モデル未登録               |
| NPU008 | Provider初期化失敗        |
| NPU009 | メモリ不足                |
| NPU010 | 設定ファイル不正             |

### 13.2 エラーレスポンス

```json
{
  "error": {
    "code": "NPU003",
    "message": "Unsupported operator detected for QNN provider",
    "provider": "qnn",
    "fallback_attempted": true,
    "fallback_provider": "cpu"
  }
}
```

---

## 14. メトリクス仕様

### 14.1 推論メトリクス

記録する項目：

* model name
* model version
* provider
* latency_ms
* fallback_used
* input_size
* output_size
* success
* error_code
* timestamp

### 14.2 Runtimeメトリクス

記録する項目：

* loaded models
* active sessions
* memory usage
* total requests
* failed requests
* fallback count

### 14.3 出力形式

MVPではJSON出力に対応する。

将来対応：

* Prometheus
* OpenTelemetry
* Grafana Dashboard

---

## 15. セキュリティ

### 15.1 MVPでの方針

MVPではローカル利用を前提とし、外部公開は想定しない。

HTTP Serverはデフォルトで以下にbindする。

```text
127.0.0.1
```

### 15.2 将来対応

将来は以下に対応する。

* API Key
* mTLS
* OAuth2
* モデル署名検証
* モデルSHA256検証
* アクセスログ
* 監査ログ

---

## 16. 非機能要件

### 16.1 性能

MVP目標値：

| 項目             | 目標      |
| -------------- | ------- |
| モデルロード時間       | 5秒以内    |
| Health Check応答 | 100ms以内 |
| 推論API overhead | 5ms以内   |
| fallback判定     | 1秒以内    |
| CLI doctor     | 10秒以内   |

### 16.2 安定性

* Provider初期化に失敗してもプロセス全体は落とさない
* モデルロード失敗時も他モデルの推論は継続する
* fallback無効時は明確なエラーを返す
* 推論タイムアウトを設定可能にする

### 16.3 拡張性

* Providerを追加可能な設計にする
* モデルtaskを追加可能な設計にする
* SDKとHTTP APIを分離する
* Runtime Coreをライブラリとして再利用可能にする

### 16.4 可観測性

* どのProviderで実行されたか必ず返す
* fallbackが発生したか必ず返す
* latencyを必ず返す
* エラー時に原因を分類する

---

## 17. ディレクトリ構成案

```text
npu-router/
  crates/
    npu-core/
    npu-ort/
    npu-qnn/
    npu-cli/
    npu-server/
    npu-sdk/
  examples/
    image-classification/
    object-detection/
    embedding/
  models/
    mobilenet/
    yolo11n/
  docs/
    getting-started.md
    model-format.md
    provider-qnn.md
  tests/
    integration/
    fixtures/
```

---

## 18. Rust crate構成

### 18.1 npu-core

共通型とRuntime抽象を定義する。

含むもの：

* Model
* Tensor
* InferenceRequest
* InferenceResponse
* Provider
* Error
* Metrics

### 18.2 npu-ort

ONNX Runtime連携を実装する。

含むもの：

* ONNX model load
* Session管理
* Tensor変換
* CPU EP実行

### 18.3 npu-qnn

QNN Execution Provider連携を実装する。

含むもの：

* QNN EP初期化
* Provider option設定
* QNN利用可否判定
* QNN実行

### 18.4 npu-cli

CLIを提供する。

含むもの：

* doctor
* providers
* models
* infer
* benchmark

### 18.5 npu-server

HTTP APIを提供する。

含むもの：

* REST API
* Health Check
* JSON response
* model inference endpoint

### 18.6 npu-sdk

アプリケーション向けRust SDKを提供する。

---

## 19. MVP開発マイルストーン

### Phase 0: 技術検証

目的：

QNN EPが対象PCで動作することを確認する。

完了条件：

* `npu doctor`相当の診断ができる
* ONNX RuntimeがARM64で動作する
* QNN EP初期化確認ができる
* CPU推論が動作する

### Phase 1: 最小推論Runtime

目的：

ONNXモデルをロードして推論する。

完了条件：

* MobileNetまたはResNetが実行できる
* Rustから推論できる
* CPU providerで動作する
* 推論結果をJSONで返せる

### Phase 2: QNN Provider対応

目的：

QNN EPでNPU推論を実行する。

完了条件：

* QNN providerを選択できる
* QNNで推論できる
* 失敗時にCPU fallbackできる
* 使用Providerをレスポンスに含める

### Phase 3: CLI / HTTP API

目的：

ミドルウェアとして利用可能にする。

完了条件：

* `npu doctor`
* `npu providers`
* `npu models`
* `npu infer`
* `npu benchmark`
* HTTP `/v1/infer/{model}`

が動作する。

### Phase 4: Vision PoC

目的：

実用ユースケースで検証する。

完了条件：

* YOLO系軽量モデルを登録できる
* 画像から物体検出できる
* latencyとfallback結果を確認できる
* 工場見守りAIの最小デモが作れる

---

## 20. 受け入れ基準

MVPは以下を満たしたら完了とする。

### 20.1 必須条件

* Windows ARM64上でビルドできる
* Snapdragon X PC上で起動できる
* `npu doctor`が環境情報を表示できる
* ONNXモデルを登録できる
* Rust SDKから推論できる
* HTTP APIから推論できる
* QNN Providerを選択できる
* QNN失敗時にCPU fallbackできる
* 推論レスポンスにproviderとlatencyが含まれる
* エラーコードが定義されている

### 20.2 推奨条件

* MobileNet / ResNetでベンチマークできる
* YOLO軽量モデルで物体検出できる
* 推論ログをJSON出力できる
* fallback発生率を確認できる
* モデル設定をYAMLで管理できる

---

## 21. 将来拡張

### 21.1 Semantic Event Bus

NPU推論結果をそのまま返すだけでなく、意味イベントとして上位アプリケーションに配信する。

例：

```text
PersonDetected
ForkliftDetected
AMRDetected
WorkerApproaching
CollisionRisk
CongestionRisk
```

### 21.2 Agent Integration

AIエージェントがNPU Routerをツールとして利用できるようにする。

対応候補：

* Hermes Agent
* OpenAI Agents
* LangGraph
* CrewAI
* MCP Server

### 21.3 Local RAG Runtime

NPU Router上でEmbedding、Reranker、小型LLMを組み合わせ、ローカルRAG基盤を提供する。

### 21.4 Model Optimization Pipeline

ONNXモデルをNPU向けに検証・最適化する機能を追加する。

対応候補：

* model validation
* quantization check
* unsupported op check
* shape check
* benchmark report
* provider compatibility report

---

## 22. 成功指標

### 22.1 技術指標

* QNN EPで推論成功
* CPU比でlatency改善
* fallback時にアプリが停止しない
* Rust SDKから簡単に利用できる
* CLIで環境診断できる

### 22.2 プロダクト指標

* 初回セットアップが30分以内
* サンプル推論が5分以内に実行可能
* QNN / CPUの違いをユーザーが意識せず使える
* エラー原因が診断可能
* 他アプリに組み込みやすい

---

## 23. まとめ

NPU Routerは、Windows ARM PCのNPUをアプリケーションから簡単に利用するための推論ミドルウェアである。

MVPでは、Snapdragon Xシリーズを対象に、ONNX RuntimeとQNN Execution Providerを利用し、Rust SDK、HTTP API、CLI診断、CPU fallbackを提供する。

初期ターゲットはLLM本体ではなく、画像分類、物体検出、Embedding、RerankerなどのNPUと相性がよい推論タスクとする。

将来的には、Semantic Event Bus、Agent連携、Local RAG Runtimeへ拡張し、工場・物流・ロボティクス向けのエッジAI基盤へ発展させる。
