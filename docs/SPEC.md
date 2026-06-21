# NPU Router 機能仕様書

| 項目 | 内容 |
| --- | --- |
| 文書版 | 0.1.0 |
| 状態 | Architecture Review Ready |
| 原案 | `docs/IDEA.md` v0.2 |
| 対象 | MVP |
| 更新日 | 2026-06-21 |

## 1. Capability

NPU Routerは、Windows 11 ARM64上のアプリケーション開発者に対し、ONNXモデルを単一のRust APIまたはローカルHTTP APIから実行する能力を提供する。実行時はQNN Execution Providerを優先し、実行基盤に起因する失敗時のみCPU Execution Providerへ明示的にfallbackする。呼び出し側はQNN SDKやONNX RuntimeのProvider設定を直接扱わず、実際に使用したProvider、fallbackの有無、レイテンシ、分類済みエラーを必ず取得できる。

### 1.1 成功状態

- 対応PCで、登録済みの静的shape ONNXモデルをQNN NPU上で推論できる。
- QNNが利用できない場合でも、許可されたモデルはCPUで推論を継続できる。
- QNNで実行したという結果に、暗黙のCPU実行が混在しない。
- Rust SDK、CLI、HTTP APIが同じCore APIと同じモデル状態を利用する。
- 環境不備、モデル不適合、入力不正、Provider障害を識別して診断できる。

## 2. スコープ

### 2.1 MVPに含む

- Windows 11 ARM64、Snapdragon X Plus / Eliteの実機対応
- ONNX Runtime C APIを利用したCPU EPおよびQNN EP
- 静的shapeのONNXモデル
- ファイルベースのモデル登録とバージョン解決
- モデルのload、unload、warmup、infer
- QNNからCPUへの明示的fallback
- Rust SDK
- localhost限定HTTP API
- `doctor`、`providers`、`models`、`model`、`infer`、`benchmark` CLI
- 構造化ログとプロセス内メトリクス
- 画像分類、物体検出、Embeddingの拡張可能な前後処理契約

### 2.2 MVPに含まない

- DirectML、Windows ML、AMD/Intel NPU、Linux、Android
- 動的shapeの一般対応
- モデル変換、自動量子化、自動最適化
- LLMのトークン生成、KV Cache管理
- 複数プロセスまたは複数ノードでのモデル状態共有
- リモートモデル取得、モデルマーケットプレイス
- 外部ネットワーク公開、認証、認可、TLS
- SLAを伴う永続メトリクス、Prometheus、OpenTelemetry
- 動画デコード、カメラ取り込み、イベントバス

## 3. 固定方針と不変条件

### 3.1 固定方針

1. QNN SDKを直接抽象化せず、ONNX Runtime QNN EPをProvider境界とする。
2. RustからONNX Runtimeへは、機能不足時の回避経路を確保できるC API境界を用いる。Rustラッパーの採用有無は実装詳細とする。
3. QNN SessionではCPU EPへの暗黙fallbackを無効にする。
4. CPU fallbackは独立したCPU Sessionを用いた、Router管理下の別試行とする。
5. モデル登録情報は設定ファイルを正とし、MVPではDBを持たない。
6. HTTP Serverは既定で`127.0.0.1`にのみbindする。
7. モデルのパスはモデルルート配下に制限し、path traversalとsymlink escapeを拒否する。

### 3.2 不変条件

- 成功レスポンスは`provider`、`fallback_used`、`latency_ms`、`request_id`を必ず含む。
- `provider=qnn`は、その試行でCPU EPが使用されていないことを意味する。
- 入力検証エラー、shape不一致、dtype不一致はfallbackしない。
- 同一の`name + version + provider + runtime configuration`は同一Session keyを持つ。
- モデルの設定またはファイルが変わっても、既存Sessionを暗黙に差し替えない。再loadを必要とする。
- モデル単位の失敗が他モデルのSessionを破棄してはならない。
- 秘密情報、入力本文、base64画像、Tensor値を既定ログへ出力しない。

## 4. 対象環境

| 項目 | 必須条件 |
| --- | --- |
| OS | Windows 11 ARM64 |
| CPU/NPU | Snapdragon X PlusまたはSnapdragon X Elite / Qualcomm Hexagon NPU |
| Runtime | ONNX Runtime ARM64 shared library |
| NPU Provider | QNN Execution Providerおよび互換QNN backend library |
| 開発言語 | Rust stable、MSVC ARM64 target |
| モデル | ONNX、固定batch、静的shape |
| HTTP | loopback TCP |

ONNX Runtime、QNN SDK、QNN backendの互換バージョンはリリースごとにsupport matrixとして固定する。MVP実装開始前に最初の組合せをADRへ記録する。

## 5. アクターと利用面

| アクター | 利用面 | 目的 |
| --- | --- | --- |
| Rustアプリケーション | `npu-sdk` | 同一プロセスで低オーバーヘッド推論 |
| ローカルアプリケーション | HTTP API | 言語非依存の推論 |
| 開発者・運用者 | CLI | 診断、登録、load、推論、ベンチマーク |
| モデル提供者 | Model manifest | 入出力、前後処理、Providerポリシーの宣言 |

## 6. 論理アーキテクチャ

```text
Rust Application       CLI                Local HTTP Client
       |                |                         |
       +----------------+-------------------------+
                        |
                 Application Facades
              npu-sdk / npu-cli / npu-server
                        |
                 NpuRouter Core API
                        |
       +----------------+----------------+
       |                |                |
 Model Registry   Inference Router   Diagnostics
       |                |                |
 Manifest Store   Session Manager   Environment Probe
                        |
             +----------+----------+
             |                     |
      QNN Provider Adapter    CPU Provider Adapter
             |                     |
      QNN-only ORT Session    CPU-only ORT Session
             +----------+----------+
                        |
              ONNX Runtime C API

 Cross-cutting: structured logging, metrics, request IDs, configuration
```

### 6.1 Crate構成

```text
crates/
  npu-core/       # ドメイン型、Router、Registry、Session lifecycle
  npu-ort/        # ORT C API安全ラッパー、Tensor変換、共通Session実装
  npu-qnn/        # QNN EP options、availability probe、QNN Session factory
  npu-tasks/      # 前処理・後処理の組込みtask実装
  npu-sdk/        # 公開Rust API。npu-coreの安定したfacade
  npu-cli/        # CLI binary
  npu-server/     # localhost HTTP server binary
```

`npu-qnn`はQNN SDKの独自推論APIを呼ばない。QNN EP固有のSession options、DLL探索、診断だけを所有する。

### 6.2 依存方向

```text
npu-cli ------> npu-sdk ------> npu-core <------ npu-server
                                  |   |
                                  |   +------> npu-tasks
                                  +----------> npu-ort <------ npu-qnn
```

- `npu-core`はHTTP、CLI、QNN固有の型に依存しない。
- `npu-ort`は`npu-core`のProvider traitを実装する。
- `npu-qnn`は`npu-ort`のSession factory拡張として実装する。
- API DTOとドメイン型を分離し、HTTP都合をSDKへ漏らさない。

## 7. Coreドメイン契約

### 7.1 主要型

```rust
pub struct ModelId {
    pub name: String,
    pub version: Version,
}

pub enum ProviderKind { Qnn, Cpu }

pub struct InferenceRequest {
    pub request_id: RequestId,
    pub model: ModelSelector,
    pub input: InferenceInput,
    pub provider: ProviderPreference,
    pub timeout: Duration,
}

pub struct InferenceResponse {
    pub request_id: RequestId,
    pub model: ModelId,
    pub provider: ProviderKind,
    pub fallback_used: bool,
    pub latency_ms: f64,
    pub result: TaskOutput,
}
```

公開APIでは生の`OrtValue`やQNN handleを返さない。

### 7.2 Provider契約

```rust
pub trait ProviderFactory: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn probe(&self) -> ProviderStatus;
    fn create_session(
        &self,
        model: &ResolvedModel,
        options: &SessionConfig,
    ) -> Result<Arc<dyn InferenceSession>, NpuError>;
}

pub trait InferenceSession: Send + Sync {
    fn metadata(&self) -> &SessionMetadata;
    fn run(&self, inputs: TensorMap) -> Result<TensorMap, NpuError>;
}
```

実装時に利用するRust bindingが`Send`または`Sync`を保証しない場合、Sessionごとの専用worker threadとbounded channelで隔離し、このtraitの外部契約を維持する。

### 7.3 Task契約

```rust
pub trait TaskProcessor: Send + Sync {
    fn task_type(&self) -> TaskType;
    fn preprocess(&self, input: InferenceInput, spec: &InputSpec)
        -> Result<TensorMap, NpuError>;
    fn postprocess(&self, output: TensorMap, spec: &OutputSpec)
        -> Result<TaskOutput, NpuError>;
}
```

MVP組込みtaskは`image_classification`、`object_detection`、`embedding`とする。ただしPhase 1では`image_classification`のみを実装し、他taskは段階的に追加できる。

## 8. モデルRegistry

### 8.1 保存方式

- 既定のモデルルートは`./models`。
- 1モデルバージョンにつき1つのYAML manifestを持つ。
- 起動時scanと明示的`model register`の双方を許可する。
- registerはmanifestを検証し、管理対象ディレクトリへatomic copyする。
- Registryはmanifestの絶対パス、モデルファイルSHA-256、更新時刻を保持する。
- MVPの`register`は実行中プロセスのRegistryとファイルシステムを更新するが、自動loadはしない。

### 8.2 ディレクトリ形式

```text
models/
  person-detector/
    0.1.0/
      model.yaml
      model.onnx
      labels.txt
```

### 8.3 Manifest schema

```yaml
schema_version: 1
name: person-detector
version: 0.1.0
format: onnx
model_path: model.onnx
model_sha256: "<64 lowercase hex chars>" # MVPでは任意、指定時は必ず検証

task: object_detection

providers:
  preferred: qnn
  fallback: [cpu]
  fail_fast: false
  qnn:
    backend: htp
    options: {}

session:
  warmup_runs: 1
  max_concurrency: 1
  timeout_ms: 30000

inputs:
  - name: images
    dtype: float32
    shape: [1, 3, 640, 640]
    layout: nchw

outputs:
  - name: output0
    dtype: float32

preprocess:
  resize: { width: 640, height: 640, mode: letterbox }
  color: rgb
  normalize:
    scale: 0.00392156862745098
    mean: [0.0, 0.0, 0.0]

postprocess:
  type: yolo
  labels_path: labels.txt
  confidence_threshold: 0.25
  nms_threshold: 0.45
```

### 8.4 Manifest検証

登録時に以下を拒否する。

- 未対応`schema_version`、`format`、`task`、`dtype`、`layout`
- SemVerとして不正なversion
- 重複するinput/output名
- 0以下または動的値を含むshape
- モデルルート外を参照するpath
- 存在しないモデルまたは補助ファイル
- SHA-256指定時の不一致
- `preferred`とfallbackの重複
- 未対応Provider
- `fail_fast: true`と空でないfallbackの矛盾

### 8.5 バージョン解決

- `name@version`指定時は完全一致を使用する。
- version省略時は、登録済みの非pre-release SemVerの最大値を使用する。
- 利用可能なversionがpre-releaseのみの場合は、明示指定を必須とする。
- 解決したversionは1リクエスト中に変更しない。

## 9. モデルとSessionの状態

### 9.1 状態

```text
UNLOADED -> LOADING -> LOADED
               |         |
               v         v
             FAILED <- UNLOADING -> UNLOADED

REGISTEREDはRegistry上の属性であり、Session状態とは分離する。
DISABLEDは運用属性であり、loadとinferを拒否する。
```

### 9.2 遷移規則

| 操作 | 事前状態 | 成功 | 失敗 |
| --- | --- | --- | --- |
| load | UNLOADED / FAILED | LOADED | FAILED |
| infer | LOADED | LOADED | LOADED。致命的Session障害時のみFAILED |
| unload | LOADED / FAILED | UNLOADED | 元状態を維持 |
| reload | 任意 | 新SessionをLOADED後にatomic swap | 旧Sessionを維持 |

- 同じSession keyへの同時loadはsingle-flight化する。
- unload中の新規inferは`NPU021 MODEL_UNLOADING`を返す。
- 実行中inferは完了を待ってからSessionを破棄する。
- failed sessionは自動無限再生成しない。明示loadまたは設定された有限回retryのみ許可する。

## 10. Session Manager

### 10.1 Session key

```text
(model name, model version, model SHA-256, provider, normalized session options)
```

### 10.2 キャッシュ

- プロセス内のbounded LRU cacheとする。
- 既定上限は同時load 4 Session。設定可能とする。
- 実行中、pin中、load中のSessionはevictしない。
- evict時は新規要求から切り離し、in-flight countが0になってから破棄する。
- QNN SessionとCPU Sessionは別entryとする。

### 10.3 並行実行

- モデルmanifestの`max_concurrency`をSession単位の上限とする。
- 超過要求はbounded queueで待機する。
- queue待機を含めてrequest deadlineを適用する。
- queue満杯時は`NPU020 BUSY`を返し、fallbackしない。
- MVP既定値はQNN=1、CPU=`min(logical_cpu_count, 4)`とする。

## 11. Provider選択とfallback

### 11.1 選択アルゴリズム

```text
1. manifestとリクエストからProvider chainを解決する
2. リクエストとmanifestを検証する
3. chainの先頭Providerについてavailabilityを確認する
4. 専用Sessionを取得または生成する
5. 推論する
6. retryable provider errorの場合のみ次Providerへ進む
7. 成功またはchain枯渇で終了する
```

### 11.2 Provider指定

| 指定 | 動作 |
| --- | --- |
| `auto` | manifestのpreferredとfallbackを使用 |
| `qnn` | QNNのみ。CPUへfallbackしない |
| `cpu` | CPUのみ |

CLI benchmarkは既定でProviderを明示し、異なるProviderへのfallbackを禁止する。

### 11.3 fallback対象

| エラー分類 | fallback | 理由 |
| --- | --- | --- |
| Provider利用不可・初期化失敗 | する | 別Providerで回復可能 |
| QNN Session生成失敗・未対応operator | する | CPU Sessionで回復可能 |
| QNN実行時Provider failure | する | 入力が正しければCPU再試行可能 |
| Provider側OOM | する | CPU memoryで回復する可能性あり |
| 入力形式・dtype・shape不正 | しない | Providerを変えても要求は不正 |
| モデル未登録・disabled | しない | Providerと無関係 |
| 前処理・後処理エラー | しない | Providerと無関係 |
| queue満杯 | しない | 負荷制御を迂回しない |
| request deadline超過 | しない | 再試行すると期限をさらに超える |
| 内部不変条件違反 | しない | fail closed |

### 11.4 タイムアウト

MVPのtimeoutは呼び出し側deadlineである。ONNX Runtimeの実行を安全に中断できない構成では、deadline後にレスポンスを破棄してもworker処理は終了まで継続し得る。この場合も同じSessionへ無制限に処理を追加せず、占有中として扱う。強制cancelはMVPの保証対象外とする。

## 12. 推論処理フロー

```text
Client
  | request
  v
Facade -> Request validation -> Model/version resolution
  |                                  |
  |                                  v
  |                            Task preprocess
  |                                  |
  |                                  v
  |                            Provider chain
  |                                  |
  |                  +---------------+---------------+
  |                  v                               v
  |            QNN-only Session --retryable--> CPU-only Session
  |                  |                               |
  |                  +---------------+---------------+
  |                                  v
  |                            Task postprocess
  |                                  |
  v                                  v
Response <- metrics/logging <- provider + fallback + latency
```

レイテンシを以下に分けて計測する。

- `queue_ms`
- `preprocess_ms`
- `inference_ms`
- `postprocess_ms`
- `total_ms`

公開レスポンスの`latency_ms`は`total_ms`とする。

## 13. Rust SDK仕様

### 13.1 初期化

```rust
let router = NpuRouter::builder()
    .model_dir("./models")
    .max_loaded_sessions(4)
    .default_timeout(Duration::from_secs(30))
    .build()?;
```

build時はRegistry scanとProvider probeを行うが、モデルをloadしない。QNNが利用不可でも、CPUが利用可能なら初期化は成功する。

### 13.2 操作

```rust
router.register_model("./incoming/model.yaml")?;
router.load_model("person-detector@0.1.0")?;

let response = router
    .model("person-detector")
    .provider(ProviderPreference::Auto)
    .timeout(Duration::from_secs(10))
    .infer(input)?;

router.unload_model("person-detector@0.1.0")?;
```

### 13.3 SDK互換性

- 公開型はSemVerに従う。
- Provider固有optionは型付き共通設定と、`experimental` namespaceを分離する。
- `npu-core`の内部型をSDKの公開APIとして再exportしない。

## 14. HTTP API仕様

### 14.1 共通

- Base path: `/v1`
- Content-Type: `application/json`
- bind既定値: `127.0.0.1:8080`
- request body上限: 16 MiB
- 同時HTTP request上限: 32
- client指定またはserver生成の`X-Request-Id`をレスポンスへ返す。
- CORSは既定で無効。

### 14.2 Endpoint

| Method | Path | 機能 |
| --- | --- | --- |
| GET | `/health/live` | プロセス生存確認 |
| GET | `/health/ready` | Registryと最低1 Providerの利用可否 |
| GET | `/v1/providers` | Provider診断結果 |
| GET | `/v1/models` | 登録モデル一覧 |
| GET | `/v1/models/{name}` | versionと状態一覧 |
| POST | `/v1/models/{name}/{version}/load` | Session load |
| POST | `/v1/models/{name}/{version}/unload` | Session unload |
| POST | `/v1/infer/{name}` | version自動解決で推論 |
| POST | `/v1/infer/{name}/{version}` | version指定で推論 |
| GET | `/v1/metrics` | JSON snapshot |

モデル登録は任意ファイルパスを受け取るため、MVPのHTTP APIでは提供せずCLIまたはSDKに限定する。

### 14.3 推論request

```json
{
  "provider": "auto",
  "timeout_ms": 10000,
  "input": {
    "type": "image_base64",
    "media_type": "image/jpeg",
    "data": "..."
  }
}
```

- `provider`省略時は`auto`。
- `timeout_ms`は1以上、server上限以下。
- base64 decode後サイズもbody上限に含める。
- 対応画像形式はJPEG、PNG。content sniffingを行い、宣言との不一致を拒否する。

### 14.4 成功response

```json
{
  "request_id": "01J...",
  "model": { "name": "person-detector", "version": "0.1.0" },
  "provider": "qnn",
  "fallback_used": false,
  "latency_ms": 22.4,
  "timing": {
    "queue_ms": 0.2,
    "preprocess_ms": 1.8,
    "inference_ms": 18.7,
    "postprocess_ms": 1.7
  },
  "result": {
    "type": "object_detection",
    "objects": [
      { "label": "person", "score": 0.91, "bbox": [120, 80, 220, 310] }
    ]
  }
}
```

### 14.5 HTTP status mapping

| Status | 条件 |
| --- | --- |
| 200 | 推論成功。fallback成功も200 |
| 400 | request、入力、manifest由来の検証エラー |
| 404 | モデルまたはversion未登録 |
| 409 | model disabled、loading、unloadingなど状態競合 |
| 413 | bodyまたはdecode後入力が上限超過 |
| 422 | task入力として解釈不能、shape/dtype不一致 |
| 429 | concurrencyまたはqueue上限超過 |
| 503 | Provider利用不可、全fallback失敗 |
| 504 | request deadline超過 |
| 500 | 内部エラー |

## 15. CLI仕様

```text
npu doctor [--json] [--run-smoke-test]
npu providers [--json]
npu models [--json]
npu model register <manifest-path>
npu model validate <manifest-path> [--provider qnn|cpu]
npu model load <name[@version]> [--provider auto|qnn|cpu]
npu model unload <name[@version]>
npu infer <name[@version]> --input <path> [--provider auto|qnn|cpu] [--json]
npu benchmark <name[@version]> --input <path> --provider <qnn|cpu>
              [--warmup 10] [--runs 100] [--json]
npu serve [--bind 127.0.0.1:8080]
```

- 人間向け出力はstderrへ進捗、stdoutへ結果を出す。
- `--json`はstdoutへ単一JSON documentを出し、ログを混在させない。
- 終了コードは0=成功、2=入力/設定不正、3=環境不備、4=Provider失敗、5=推論失敗、10=内部エラーとする。
- benchmarkはwarmupを統計から除外し、fallbackを禁止する。
- benchmarkは平均、median、p95、p99、min、max、throughput、成功件数を返す。

## 16. Diagnostics仕様

### 16.1 `doctor`検査項目

| ID | 検査 | 必須 |
| --- | --- | --- |
| D001 | Windows version | yes |
| D002 | process architectureがARM64 | yes |
| D003 | CPU/NPU識別情報 | yes |
| D004 | ONNX Runtime DLL load | yes |
| D005 | ONNX Runtime API version | yes |
| D006 | 利用可能EP一覧にQNN/CPUが存在 | yes |
| D007 | QNN backend DLL探索とload | QNN利用時yes |
| D008 | QNN Session option検証 | QNN利用時yes |
| D009 | CPU smoke model load/infer | `--run-smoke-test`時 |
| D010 | QNN smoke model load/infer | `--run-smoke-test`時 |

各検査は`pass`、`warn`、`fail`、`skipped`と、修正可能なmessageを返す。QNN failかつCPU passはRouter全体として`degraded`、両方failは`not_ready`とする。

## 17. エラー契約

### 17.1 エラー形式

```json
{
  "request_id": "01J...",
  "error": {
    "code": "NPU013",
    "category": "provider",
    "message": "QNN session creation failed",
    "provider": "qnn",
    "retryable": true,
    "fallback_attempted": true,
    "fallback_provider": "cpu",
    "details": {}
  }
}
```

`details`は安全な構造化情報だけを含み、DLL絶対パス、環境変数値、入力内容を外部レスポンスへ出さない。

### 17.2 エラーコード

| Code | 意味 | Retryable |
| --- | --- | --- |
| NPU001 | QNN unavailable | yes |
| NPU002 | model load failed | depends |
| NPU003 | unsupported operator/provider incompatibility | yes, other provider only |
| NPU004 | input shape mismatch | no |
| NPU005 | request deadline exceeded | no |
| NPU006 | provider chain exhausted | no |
| NPU007 | model/version not registered | no |
| NPU008 | provider initialization failed | yes |
| NPU009 | provider out of memory | yes, other provider only |
| NPU010 | invalid manifest/configuration | no |
| NPU011 | input dtype/layout mismatch | no |
| NPU012 | preprocessing failed | no |
| NPU013 | session creation failed | depends |
| NPU014 | inference execution failed | depends |
| NPU015 | postprocessing failed | no |
| NPU016 | model disabled | no |
| NPU017 | model checksum mismatch | no |
| NPU018 | unsafe model path | no |
| NPU019 | provider explicitly required but unavailable | no |
| NPU020 | router busy / queue full | yes |
| NPU021 | model state conflict | yes |
| NPU022 | unsupported task or schema version | no |
| NPU999 | internal invariant violation | no |

## 18. セキュリティと信頼境界

### 18.1 信頼境界

```text
Untrusted local HTTP input
        |
        v
HTTP limits and decoding ----> validated TaskInput
                                      |
Trusted operator manifest ---------->|
                                      v
                           Native ORT/QNN boundary
```

### 18.2 必須対策

- loopback以外へのbindは`--allow-remote`明示時でもMVPでは拒否する。
- request body、画像dimension、Tensor element数、queue長に上限を持つ。
- 整数overflowを検査してからTensor bufferを確保する。
- YAML alias expansionと過剰nestingを制限する。
- モデルと補助ファイルをcanonicalizeし、モデルルート配下であることを確認する。
- DLL探索順序を固定し、current working directoryから任意DLLをloadしない。
- manifestのQNN optionsはallowlist方式とし、未知keyを拒否する。
- native error textをそのままHTTPへ返さない。
- SHA-256指定モデルはload前に毎回、または検証済みmetadata cacheに基づき検査する。

## 19. 可観測性

### 19.1 構造化ログ

最低限以下をJSON Linesで記録する。

- timestamp、level、event、request_id
- model name/version
- requested_provider、actual_provider
- fallback_used、fallback_reason_code
- queue/preprocess/inference/postprocess/total latency
- success、error_code

入力内容、出力Tensor、base64、個人情報になり得るlabel値は既定で記録しない。

### 19.2 メトリクス

- requests total / failed / timed out
- fallback total（source、destination、reason別）
- latency histogram（model、provider別）
- queue wait、queue rejection
- Session load time、load failure、loaded Session数
- Provider availabilityと最終probe時刻
- process working set

model versionは許容するがrequest IDやerror messageをmetric labelにしない。

## 20. 非機能要件

### 20.1 性能目標

| 指標 | 条件 | 目標 |
| --- | --- | --- |
| `/health/live` | idle、localhost | p95 100 ms以下 |
| Router overhead | load済み、前後処理除外 | p95 5 ms以下 |
| cold model load | MVP sample model | 5秒以下を目標、実測を記録 |
| `doctor` | smoke testなし | 10秒以下 |
| fallback決定 | QNN availability failure | 1秒以下 |

モデル依存の推論時間には固定SLOを置かない。sample modelごとにCPU/QNN基準値をrelease artifactとして記録する。

### 20.2 信頼性

- 不正入力でprocessがpanicまたはabortしない。
- Provider初期化失敗でRouter全体を終了しない。
- 1モデルのload失敗後も他モデルが推論可能である。
- panicはHTTP境界とworker境界で捕捉し、Session汚染時はFAILEDへ遷移させる。
- shutdown時は新規受付を停止し、設定時間だけin-flightをdrainする。

### 20.3 配布

- ARM64 native binaryとして配布する。
- ONNX RuntimeとQNN関連binaryのライセンス、再配布可否、versionを配布manifestへ記載する。
- DLL同梱方式とユーザー環境参照方式は実装開始前の未解決事項とする。

## 21. 設定

プロセス設定の優先順位は`CLI option > environment variable > config file > default`とする。

```yaml
server:
  bind: 127.0.0.1:8080
  max_body_bytes: 16777216
  max_concurrent_requests: 32

models:
  root: ./models
  max_loaded_sessions: 4

runtime:
  default_timeout_ms: 30000
  queue_capacity_per_session: 8
  ort_library_path: null
  qnn_backend_path: null

logging:
  format: json
  level: info
```

環境変数は`NPU_ROUTER_` prefixを使う。secretを必要とする設定はMVPに存在しない。

## 22. 検証戦略

### 22.1 Unit test

- manifest parse、全validation rule、path containment
- SemVer latest解決
- Provider chain生成
- retryable/non-retryable分類
- state machineの全遷移
- Session key正規化
- queueとdeadline
- HTTP DTOとerror mapping
- 前後処理のgolden vector

### 22.2 Integration test

- CPU EPでsample model load/infer/unload
- QNNをmockしたavailability、session failure、runtime failure
- QNN失敗からCPU成功へのfallback
- 入力不正時にCPUを呼ばないこと
- concurrent loadのsingle-flight
- reload失敗時に旧Sessionが継続すること
- body/queue/concurrency上限
- graceful shutdown

### 22.3 Windows ARM64実機test

- QNN専用Sessionでsample modelが成功すること
- QNN SessionでCPU EP fallbackが無効であること
- QNN DLL欠落時にdoctorが修正可能な診断を返すこと
- QNN非対応modelがCPUへfallbackすること
- QNNとCPUの出力が定義済み許容誤差内で一致すること
- 100回benchmarkでcrash、resource leak、Session増加がないこと

### 22.4 Test fixture

- tiny classification ONNX model
- QNN対応の量子化済み静的shape model
- QNN非対応operatorを含むmodel
- shape、dtype、checksum、pathが不正なmanifest群
- 既知入力と期待出力のgolden data

## 23. 受け入れ基準

### AC-01 Environment

- 対象Windows ARM64 PCでrelease buildが起動する。
- `npu doctor --run-smoke-test --json`がCPUとQNNを個別判定する。
- 必須DLL欠落時、process crashではなく該当diagnostic IDを返す。

### AC-02 Registry

- 有効なmanifestを登録し、再起動後も一覧へ表示できる。
- 不正shape、root外path、checksum不一致を登録時に拒否する。
- version省略時の解決結果がSemVer規則と一致する。

### AC-03 Inference

- Rust SDKとHTTP APIから同じsample modelを推論できる。
- 成功結果にrequest ID、解決version、Provider、fallback、全体レイテンシが含まれる。
- QNN成功時にQNN専用Sessionが使われる。

### AC-04 Fallback

- QNNのavailability、Session生成、実行障害でCPUへ一度だけfallbackする。
- 入力shape不正ではCPUを試行しない。
- `provider=qnn`または`fail_fast=true`ではCPUを試行しない。
- 全Provider失敗時、各attemptを内部ログに残し、外部へ`NPU006`を返す。

### AC-05 Isolation and load

- 1モデルをFAILEDにしても別モデルの推論が成功する。
- queue上限超過時に429 / `NPU020`を返す。
- unloadはin-flight requestを破壊せず、新規requestを拒否する。

### AC-06 Benchmark and observability

- QNNとCPUを別々に100回測定し、平均、p95、p99、throughputをJSONで取得できる。
- 全推論にprovider、fallback、latencyの構造化ログが存在する。
- 入力画像またはTensor値が標準ログに存在しない。

## 24. 開発フェーズとgate

### Phase 0: 技術spike

- ORT/QNN/QNN backendの互換versionを固定する。
- Windows ARM64でC APIからCPU/QNN Sessionを生成する。
- QNN内CPU fallbackを無効化できることを確認する。
- Rust FFI/bindingの`Send`/`Sync`とcancel挙動を検証する。

Gate: ADR-001 Runtime integration、ADR-002 binary distribution、実機ログを承認する。

### Phase 1: Core + CPU

- Registry、manifest validation、Session Manager、classification task
- CPU Provider、Rust SDK、unit/integration test

Gate: CPU受け入れ基準とunsafe境界reviewを通過する。

### Phase 2: QNN + fallback

- QNN probe、QNN Session factory、provider chain、doctor
- QNN実機test、CPUとの精度比較

Gate: AC-01、AC-04とQNN専用実行の証跡を満たす。

### Phase 3: CLI + HTTP

- CLI全操作、HTTP endpoints、limits、graceful shutdown
- JSON loggingとmetrics snapshot

Gate: AC-03、AC-05、security testを満たす。

### Phase 4: Vision PoC

- object detection task、YOLO後処理、benchmark report
- 工場見守りの静止画demo

Gate: sample modelで再現可能なセットアップ手順と測定結果を公開する。

## 25. 未解決事項

以下は実装開始前またはPhase gateまでに決定が必要である。

| ID | 決定事項 | 期限 | 影響 |
| --- | --- | --- | --- |
| OQ-01 | ONNX Runtime、QNN SDK、backendの固定version | Phase 0開始前 | ABI、機能、配布 |
| OQ-02 | ORT/QNN DLLを同梱するか、別途導入を要求するか | Phase 0 gate | インストール、ライセンス |
| OQ-03 | Rust bindingを採用するか、最小C API wrapperを自作するか | Phase 0 gate | unsafe範囲、EP option対応 |
| OQ-04 | QNN対象の基準modelと量子化形式 | Phase 0開始前 | 技術成立性、性能評価 |
| OQ-05 | QNN実行timeout/cancelの実挙動 | Phase 0 gate | worker隔離、回復性 |
| OQ-06 | 同一Sessionの安全な並行`Run`可否と推奨concurrency | Phase 0 gate | throughput、locking |
| OQ-07 | HTTPでEmbedding raw tensor入力を許可するか | Phase 3開始前 | API、入力上限 |
| OQ-08 | モデル署名をMVPへ前倒しする必要があるか | Phase 2 gate | supply-chain security |

## 26. Handoff

本仕様はarchitecture reviewに進める状態であり、未解決事項を残したまま全機能を一括実装する状態ではない。次の作業はPhase 0 spikeとADR-001/002の作成である。技術成立性が確認された後、`tdd-workflow`でPhase 1を縦切りし、`verification-loop`で各受け入れ基準を検証する。

## 27. 参考資料

- [ONNX Runtime QNN Execution Provider](https://onnxruntime.ai/docs/execution-providers/QNN-ExecutionProvider.html)
- [ONNX Runtime: Build with different Execution Providers](https://onnxruntime.ai/docs/build/eps.html)
- [ONNX Runtime C API](https://onnxruntime.ai/docs/api/c/)
