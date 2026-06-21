# NPU Router 実装プロンプト

`docs/SPEC.md` を正本として、指定された Issue、Phase、または受け入れ基準を実装してください。

指定例:

```text
@docs/PROMPT.md に従って Issue #<番号> を実装する。
@docs/PROMPT.md に従って Phase 0 の技術 spike を進める。
@docs/PROMPT.md に従って AC-02 Registry を実装する。
```

## 前提

- 現在は設計・技術検証段階であり、Rust workspace と実行可能な Runtime はまだ存在しない。
- MVP は Windows 11 ARM64、Snapdragon X Plus / Elite、ONNX Runtime QNN EP と CPU EP を対象とする。
- DirectML、Windows ML、AMD/Intel NPU、動的 shape、モデル自動量子化、LLM 生成は MVP の対象外とする。
- `docs/SPEC.md` の固定方針、不変条件、Phase gate、受け入れ基準を変更する必要がある場合は、実装を始める前に理由と影響を提示する。
- `docs/IDEA.md` は構想の原案であり、内容が競合する場合は `docs/SPEC.md` を優先する。

## 実装フロー

### 1. 対象と成功条件を確定する

- Issue 指定時は `gh issue view <番号>` で要件、背景、受け入れ条件を確認する。
- 対象に関係する `docs/SPEC.md` の節、AC、Phase gate、未解決事項（OQ）を列挙する。
- 実装対象外を明記し、未解決事項を推測で確定しない。
- 作業を、個別に検証可能な最小の縦切りへ分割する。
- 複雑な変更では、実装前に依存関係、リスク、検証方法を含む短い計画を提示する。

Phase 0 では特に OQ-01〜OQ-06 を確認し、次の gate を満たす証跡を成果物に含める。

- ADR-001: Runtime integration
- ADR-002: binary distribution
- Windows ARM64 実機での CPU/QNN Session 検証ログ
- QNN Session 内の暗黙 CPU fallback が無効である証跡

### 2. 既存状態を確認する

```bash
git status --short
git branch --show-current
rg --files
```

- ユーザーの未コミット変更を保持し、依頼と無関係な変更を行わない。
- ブランチ作成を依頼されている場合のみ、最新の `main` を基点に `feature/issue-<番号>-<説明>` または `feature/<phase>-<説明>` を作成する。
- 実装開始前に、対象環境と必要な SDK/DLL/モデルが利用可能か確認する。実機が必要な検証を mock で代替して完了扱いにしない。

### 3. TDD で実装する

1. **Red**: 仕様または不具合を再現する失敗テストを先に追加する。
2. **Green**: テストを通す最小限の実装を行う。
3. **Refactor**: 振る舞いを変えずに整理し、全テストを再実行する。

変更に応じて次を検証する。

- Unit: manifest validation、path containment、SemVer 解決、state machine、Provider chain、エラー分類
- Integration: CPU load/infer/unload、QNN mock、明示的 fallback、single-flight、queue/deadline
- Windows ARM64 実機: QNN 専用 Session、CPU fallback 無効化、CPU/QNN 精度比較、連続 benchmark
- CLI/HTTP: JSON 契約、status/exit code、入力上限、負荷制御、graceful shutdown

テスト fixture は `docs/SPEC.md` 22.4 に従う。テストのためだけに本番契約を弱めない。

### 4. アーキテクチャと安全性を守る

- QNN と CPU は独立した ONNX Runtime Session とし、QNN Session 内の暗黙 fallback を使用しない。
- 入力、shape、dtype、モデル状態に起因するエラーでは fallback しない。
- `npu-core` に HTTP、CLI、QNN 固有型を持ち込まない。
- unsafe/FFI 境界を最小化し、安全なラッパーに閉じ込める。所有権、lifetime、thread safety をテストまたは根拠で示す。
- モデルパスはモデルルート配下に制限し、path traversal と symlink escape を拒否する。
- HTTP は既定で `127.0.0.1` のみに bind し、body、decode 後入力、同時 request、queue を制限する。
- 外部入力は境界で検証し、native error、絶対パス、環境変数値、画像、base64、Tensor 値をレスポンスや標準ログへ出さない。
- hardcoded secret を追加しない。依存関係と native binary の version、入手元、ライセンスを記録する。
- 既存オブジェクトの破壊的な共有状態変更を避け、新しい値または atomic swap で状態遷移する。

### 5. 品質を検証する

Rust workspace 作成後は、少なくとも次を実行する。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

必要に応じて以下も実行する。

```bash
cargo test --test '<integration-test-name>'
cargo llvm-cov --workspace --all-features --fail-under-lines 80
cargo audit
```

- 変更に適用できる全テストを通し、カバレッジ 80% 以上を維持する。
- Windows ARM64/QNN 実機テストを実行できない環境では、未実施項目、理由、実行手順を明記する。
- benchmark は Provider を `qnn` または `cpu` に固定し、warmup を統計から除外する。平均、median、p95、p99、min、max、throughput、成功件数を JSON で保存する。
- ドキュメントのみの変更では、リンク、コマンド、ファイル名、仕様との整合性を確認し、不要な Rust テストは実行しない。

### 6. 差分をレビューする

```bash
git diff --check
git diff --stat
git diff
```

- すべての変更行が依頼またはテストに直接対応していることを確認する。
- correctness、security、error handling、並行性、FFI safety、fallback 条件をレビューする。
- CRITICAL/HIGH の指摘を解消し、修正後に関連テストを再実行する。

### 7. ドキュメントを更新する

- 実装が契約、CLI、HTTP API、設定、配布、運用手順を変える場合は `README.md` または `docs/SPEC.md` の該当箇所を更新する。
- Phase 0 の決定は既存のドキュメント構成に ADR として記録する。置き場所が未確定なら、新しいトップレベル文書を作る前に確認する。
- 未解決事項を決定した場合は、根拠と決定先を記録し、`docs/SPEC.md` の OQ と整合させる。
- 実装と同じ内容を複数の文書へ重複記載しない。

### 8. コミット、push、PR

コミット、push、PR 作成、マージは依頼または明示承認がある場合のみ行う。

- コミット形式: `<type>(<scope>): <description>`
- type: `feat`、`fix`、`test`、`docs`、`refactor`、`chore`、`perf`、`ci`
- PR には対象 Issue/AC/Phase、仕様上の判断、テスト結果、実機未検証項目、security/FFI 影響を記載する。
- Issue を完了する PR は本文に `Closes #<番号>` を含める。
- CI 失敗時はログから原因を特定し、テストを弱めず実装を修正する。
- マージ前に受け入れ条件と Phase gate を一項目ずつ再確認する。

## 完了報告

次の形式で簡潔に報告する。

1. 実装した内容と主な設計判断
2. 対応した `docs/SPEC.md` の節、AC、Phase gate
3. 実行した検証と結果
4. 未実施の実機検証、残るリスク、未解決事項
5. 変更したファイル

## チェックリスト

- [ ] 対象 Issue/Phase/AC と実装対象外を確定した
- [ ] 関連する固定方針、不変条件、OQ を確認した
- [ ] 失敗テストを先に追加した（コード変更時）
- [ ] Unit / Integration / E2E / 実機テストの適用範囲を判断した
- [ ] QNN と CPU が独立 Session である
- [ ] fallback 条件が `docs/SPEC.md` 11.3 と一致する
- [ ] 入力検証、path containment、ログ秘匿、loopback bind を確認した
- [ ] format、lint、test、coverage、依存関係監査を必要な範囲で実行した
- [ ] `git diff --check` と差分レビューを完了した
- [ ] 仕様または利用方法の変更をドキュメントへ反映した
- [ ] 実機未検証項目と残るリスクを明記した
- [ ] 外部操作は依頼または明示承認の範囲内で行った
