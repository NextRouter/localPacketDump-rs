# localpacketdump

eth0 上の通信量を測定し、ローカル IP ごとの送受信データを Prometheus メトリクスとして出力する Rust プログラムです。

## 機能

- eth0(または指定された NIC)でパケットをキャプチャ
- ローカル IP アドレスごとの送受信バイト数を集計
- 1 秒間隔で bps (bits per second) に変換して Prometheus メトリクスとして出力
- `http://localhost:32599/status` から NIC マッピング情報を取得し、IP と NIC の対応を管理

## メトリクス

以下のメトリクスが `http://localhost:59122/metrics` で公開されます:

- `network_ip_tx_bps{local_ip="x.x.x.x", nic="ethX"}` - IP ごとの送信 bps
- `network_ip_rx_bps{local_ip="x.x.x.x", nic="ethX"}` - IP ごとの受信 bps
- `network_ip_tx_bps_total{nic="ethX"}` - NIC ごとの合計送信 bps
- `network_ip_rx_bps_total{nic="ethX"}` - NIC ごとの合計受信 bps

## Prometheus 設定

```yaml
scrape_configs:
  - job_name: "localpacketdump"
    scrape_interval: 1s
    static_configs:
      - targets: ["localhost:59122"]
```

## セットアップ

### 前提条件

- Rust (1.70+)
- libpcap (パケットキャプチャライブラリ)
- root 権限 (パケットキャプチャに必要)

#### macOS での libpcap インストール

```bash
# Homebrew を使用
brew install libpcap
```

#### Linux での libpcap インストール

```bash
# Debian/Ubuntu
sudo apt-get install libpcap-dev

# RedHat/CentOS/Fedora
sudo yum install libpcap-devel
```

### ビルドと実行

```bash
# ビルド
cargo build --release

# テスト実行
./setup.sh test
```

### systemd サービスとしてインストール (Linux のみ)

```bash
# サービスファイルを作成してインストール
sudo ./setup.sh

# サービスを開始
sudo systemctl start localpacketdump.service

# サービスのステータスを確認
sudo systemctl status localpacketdump.service

# サービスを停止
sudo systemctl stop localpacketdump.service
```

## ローカルサブネットの設定

ローカル IP アドレスのサブネットは `src/main.rs` の定数 `LOCAL_SUBNETS` で指定します:

```rust
// ローカルサブネットの定義（CIDR 形式で指定）
const LOCAL_SUBNETS: &[&str] = &[
    "10.40.0.0/24",
    // 必要に応じて追加
    // "192.168.1.0/24",
    // "172.16.0.0/16",
];
```

この定数を編集して、監視したいローカルサブネットを指定してください。複数のサブネットを配列として指定できます。

## NIC マッピング

プログラムは `http://localhost:32599/status` から以下の形式で NIC マッピング情報を取得します:

```json
{
  "config": {
    "lan": "eth2",
    "wan0": "eth0",
    "wan1": "eth1"
  },
  "mappings": {
    "10.40.0.3": "wan1"
  }
}
```

- `mappings` に含まれる IP はそれぞれ指定された wan に割り当てられます
- `mappings` に含まれない IP は全て `wan0` に割り当てられます
- マッピング情報は 10 秒ごとに自動更新されます
- **注意**: NIC マッピングはメトリクスのラベル付けにのみ使用され、ローカル IP の判定には使用されません

## トラブルシューティング

### パーミッションエラー

パケットキャプチャには root 権限が必要です。以下のいずれかの方法で実行してください:

```bash
# sudo で実行
sudo ./target/release/localpacketdump

# または capability を設定 (Linux のみ)
sudo setcap cap_net_raw,cap_net_admin=eip ./target/release/localpacketdump
./target/release/localpacketdump
```

### NIC が見つからない

使用可能なネットワークインターフェースを確認:

```bash
# macOS
ifconfig

# Linux
ip link show
```

デフォルトでは `eth0` をキャプチャしますが、環境によってインターフェース名が異なる場合があります。

## ライセンス

MIT
