# ADR-0001: CLI・Server・SDKの状態所有モデル

| 項目 | 内容 |
| --- | --- |
| 状態 | Accepted |
| Issue | #7 (P0-06) |
| 対象SPEC | `docs/SPEC.md` 9, 10, 11, 13, 14, 15 |
| 関連 | 非goal 2.2「複数プロセスまたは複数ノードでのモデル状態共有」、固定方針 3.1-5、依存方向 6.2 |

## 背景

`docs/SPEC.md` はモデルとSessionの状態機械（9章）、Session Manager（10章）、Rust SDK（13章）、HTTP API（14章）、CLI（15章）をそれぞれ定義しているが、次の3点が未確定だった。

1. CLIは`npu model load` / `npu model unload`ごとに新規プロセスを起動する（単発process）。プロセス終了とともにメモリ上のSession状態は消える。一方でSession Managerの状態機械（9章）はプロセスが生存し続けることを前提にしている。両者をどう整合させるかが未定義だった。
2. `npu model register`（8.1）は実行中プロセスのRegistryとファイルシステムを更新するが、自動loadはしないと明記されている。すでに起動している`npu serve`が、別プロセスの`register`で追加されたモデルをいつ・どうやって認識するかが未定義だった。
3. 1モデルは複数Provider（QNN/CPU）の独立Session（10.1のSession keyはprovider単位）を持ちうる。CLI/HTTPで見える「モデルの状態」が、モデル集約状態なのかprovider別Session状態なのかが未区別だった。

MVPの非goal（2.2）は「複数プロセスまたは複数ノードでのモデル状態共有」を明示的に除外している。この制約が本ADRの決定を強く方向づける。

## 決定

### 1. 状態は常にちょうど1つのプロセスが所有する。プロセスをまたいだSession状態の同期機構は作らない。

- `npu-core`の`NpuRouter`インスタンスが、それを`build()`したプロセス内でのみSession Manager状態（9章の状態機械）とRegistryのin-memoryインデックスを所有する。
- プロセス間でSession状態を共有・同期するIPC、共有メモリ、ロックファイルは実装しない（非goal 2.2に従う）。
- 状態を複数リクエストにまたがって永続させたい利用者は、状態を所有する長命プロセスとして`npu serve`を起動し、そのHTTP API（14.2に既存の`/v1/models/{name}/{version}/load` `/unload`、`/v1/infer/...`）を状態操作の唯一の口とする。HTTP APIは「daemon/control API」の役割をすでに果たしており、CLIに専用のサーバー制御クライアント機能を追加する必要はない。

### 2. CLIの`model load` / `model unload`は、単発processに閉じた診断・検証コマンドとして再定義する。

- `npu model load <name[@version]> [--provider ...]`は、CLIプロセス内で`NpuRouter`を`build()`し、指定モデル・Providerの`load_model`を実行して結果（成功/失敗、状態、エラー分類）を報告した後にプロセスを終了する。Sessionはプロセス終了と同時に破棄される。
- この結果、`npu model load`の直後に別プロセスで`npu model unload`や`npu infer`を実行しても、先に読み込んだSessionは再利用されない。これは欠陥ではなく仕様である。単発コマンドの目的は「このモデルはこのProviderでこの環境で読み込めるか」を検証すること（doctorのD009/D010に近い診断的価値）であり、後続呼び出しのための恒久的なウォームSessionを用意することではない。
- Sessionを複数リクエスト間で温存したい場合は`npu serve`のHTTP load/unloadを使う。これは「daemon/control API方式」を選ぶのではなく、CLIとHTTP APIで役割を分離する決定である。CLIは単発診断、HTTP APIは永続状態の制御を担う。
- `npu infer` / `npu benchmark`はこの単発モデルを踏襲する。CLIプロセス内で必要なら`load`し、推論し、プロセス終了時にSessionを解放する。`benchmark`はProvider固定・fallback禁止（15章）の性質上、常にCLIプロセス内Router経由とし、HTTP経由の計測は行わない（ネットワーク往復がレイテンシ統計を歪めるため）。

### 3. `register`後、稼働中の`npu serve`への反映はプロセス再起動を要件とし、自動rescanは実装しない。

- `register`（8.1）はmanifest検証とファイルシステムへのatomic copyのみを行う。ファイルシステムが正（3.1-5）であり、実行中の他プロセスのRegistryへは伝播しない。
- 稼働中の`npu serve`に新しいモデルを認識させるには、その`npu serve`プロセスを再起動して起動時scanをやり直す。HTTP APIはmanifestの任意パスを受け取らないため（14.2の注記）rescanトリガーエンドポイントも本MVPでは提供しない。
- ファイル監視やhot reloadによる自動rescanはMVPの非goalとして扱う（YAGNI。要求されていない機能を先回りして作らない）。将来必要になれば別Issueとして起票する。

### 4. モデル集約状態とprovider別Session状態を区別して公開する。

- Registry属性（8章・9.1）: `REGISTERED`（登録済みか）と`DISABLED`（運用停止か）はモデルバージョン単位の属性であり、Session状態と独立である。
- Session状態（9章）: `UNLOADED` / `LOADING` / `LOADED` / `UNLOADING` / `FAILED`は、Session key（10.1: name + version + SHA-256 + provider + normalized session options）単位、つまりprovider別に存在する。同じモデルバージョンでもQNN SessionとCPU Sessionの状態は独立に変化しうる。
- `GET /v1/models/{name}`（14.2）と`npu models --json`は、モデルバージョンごとにRegistry属性を1つと、provider別Session状態のリストを別フィールドとして返す。単一の真偽値やenumにモデル全体の状態を丸めない。例:

  ```json
  {
    "name": "person-detector",
    "version": "0.1.0",
    "registered": true,
    "disabled": false,
    "sessions": [
      { "provider": "qnn", "state": "loaded" },
      { "provider": "cpu", "state": "unloaded" }
    ]
  }
  ```

- `load` / `unload`はこの表の1行（1 Session key）を操作する。`npu model unload <name[@version]>`（15章のCLI文法にはprovider指定がない）は、指定モデルバージョンに紐づく全provider Sessionを一括で対象にする。

### 5. `reload`は独立したコマンド・エンドポイントとして公開しない。

- Session key（10.1）はモデルSHA-256を含むため、`register`でファイルが更新されると新しいSession keyが生じる。したがって同じ`load`操作を再実行するだけで、9.2の「新SessionをLOADED後にatomic swap、旧Sessionを維持（失敗時）」が自然に成立する。
- 変更のない`load`呼び出しは既存Session keyへの参照を返す（冪等）。変更があった`load`呼び出しは新Session keyを作成し、旧Session keyは参照されなくなった後にLRU（10.2）でevictされる。
- 不変条件3.2「モデルの設定またはファイルが変わっても、既存Sessionを暗黙に差し替えない」は、Session keyの構成要素にSHA-256を含めることでAPIレベルの追加操作なしに満たされる。

## 状態一貫性シーケンス

### A. CLI単発（standalone）: `npu infer` がサーバーなしで完結する

```text
User          npu-cli(process)         npu-sdk/npu-core(in-process)     Filesystem
 |                  |                              |                        |
 | infer name@ver   |                               |                        |
 |----------------->| NpuRouter::build()            |                        |
 |                  |------------------------------>| scan models/           |
 |                  |                               |----------------------->|
 |                  |                               |<-- manifests ----------|
 |                  |<-- Registry (in-memory) ------|                        |
 |                  | load_model (single-flight)    |                        |
 |                  |------------------------------>| UNLOADED->LOADING      |
 |                  |                               |-> LOADED               |
 |                  | infer                         |                        |
 |                  |------------------------------>| run()                  |
 |                  |<-- InferenceResponse ---------|                        |
 |<-- result -------|                               |                        |
 |                  | process exit                  |                        |
 |                  | (Session破棄、状態は消える)     |                        |
```

### B. 永続状態: `npu serve` を状態の唯一の所有者として使う

```text
Client A (curl)     npu-server(long-lived process)     Client B (curl)
       |                       |                              |
       | POST .../load         |                              |
       |----------------------->| UNLOADED->LOADING->LOADED   |
       |<-- 200 OK -------------|                              |
       |                       |                              |
       |                       |         POST /v1/infer/...   |
       |                       |<------------------------------|
       |                       | 既にLOADED、single-flight不要  |
       |                       |------------------------------->|
       |                       |         200 OK (warm session)  |
       | POST .../unload        |                              |
       |----------------------->| LOADED->UNLOADING->UNLOADED |
       |<-- 200 OK -------------|                              |
```

Client AとBはプロセスをまたいでSessionを共有していない。両者とも同じ`npu-server`プロセスにHTTPでアクセスしているため、状態を所有するプロセスは常に1つ（`npu-server`）のままである。

### C. `register`後にサーバーへ反映するには再起動が必要

```text
Operator          npu-cli(process)      Filesystem/models/      npu-server(long-lived)
   |                    |                       |                        |
   | model register     |                       |                        |
   |------------------->| validate manifest      |                        |
   |                    |----------------------->| atomic copy            |
   |                    |<-- ok ------------------|                        |
   |<-- 完了 -----------|                        |                        |
   |                    |                       | (npu-serverは起動時scan|
   |                    |                       |  のみ。ファイル変化を  |
   |                    |                       |  監視しない)            |
   | npu serve 再起動     |                       |                        |
   |------------------------------------------------------------------------>|
   |                    |                       |----------------------->| scan
   |                    |                       |<-- 新モデル反映 --------|
```

## 検討した代替案

| 案 | 内容 | 不採用理由 |
| --- | --- | --- |
| CLIをserver専用クライアント化する | `model load/unload/infer`を含む全stateful操作をHTTP経由で常に`npu serve`に委譲し、CLI自身はSessionを持たない | 常時`npu serve`起動を前提にするとdoctorやbenchmarkのような単発診断・計測ユースケースに不要な運用負荷を強いる。6.2の依存方向（`npu-cli -> npu-sdk -> npu-core`）が示す直接embedding経路を使えなくなる |
| CLI-Server間でSession状態を共有する軽量IPC（例: ロックファイル、共有メモリ）を新設する | プロセスをまたいでSession状態を同期する | 非goal 2.2「複数プロセスまたは複数ノードでのモデル状態共有」に直接抵触する。KISS/YAGNIにも反する |
| ファイル監視によるサーバーの自動rescan | `models/`をwatchして`register`を即時反映する | MVPスコープ外の機能追加であり要求されていない。ファイル監視はプラットフォーム差やrace conditionの検証コストが高く、Phase 0-3のどのAC/Phase gateにも含まれない |
| `reload`を独立コマンド/endpointとして追加する | 明示的なreload操作を新設する | Session keyにSHA-256を含めることで`load`の再実行が同じ効果を持つため、API面を増やさずに済む。不要な抽象を追加しない（YAGNI） |

## 影響

- **後続Issueへの実装指針**: P1-06/P1-07（Session状態機械、single-flight load、LRU）はプロセスローカルな状態として実装してよい。プロセス間同期は設計不要。P3-02（CLI control plane）は`model load/unload`をCLIプロセス内Routerに対する単発操作として実装し、サーバーへのHTTPプロキシ機能は実装しない。P3-04（HTTP server）の`/v1/models/*/load`・`/unload`が唯一の永続状態操作の入口になる。
- **ドキュメント整合**: `docs/SPEC.md`の9-15章の記述と矛盾しない（新しい契約変更ではなく、既存記述の間隙を埋める運用上の決定）。SPEC本文の更新は不要と判断した。
- **利用者への影響**: `npu model load`はプロセス間で状態を持ち越さないため、README/CLIヘルプ文言に「単発診断コマンドであり、永続的にモデルを温める場合は`npu serve`を使うこと」を明記する必要がある（別Issueでヘルプ文言・README更新を行う）。
- **未解決事項への影響**: 新規OQは発生しない。既存OQ-01〜08とは独立。

## Non-goals（本ADRのスコープ外）

- control plane（CLIのサブコマンド実装、HTTP endpoint実装）そのもの。本ADRは設計判断のみを記録する。
- ファイル監視によるhot reload機能の設計（将来要求があれば別ADRで扱う）。
