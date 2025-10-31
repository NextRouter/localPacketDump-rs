use axum::{routing::get, Router};
use lazy_static::lazy_static;
use pcap::{Capture, Device};
use pnet::packet::ethernet::{EtherTypes, EthernetPacket};
use pnet::packet::ipv4::Ipv4Packet;
use pnet::packet::Packet;
use prometheus::{Encoder, GaugeVec, Opts, Registry, TextEncoder};
use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time;
use tracing::{error, info};

// ローカルサブネットの定義（CIDR形式で指定）
const LOCAL_SUBNETS: &[&str] = &[
    "10.40.0.0/20",
    // 必要に応じて追加
    // "192.168.1.0/24",
    // "172.16.0.0/16",
];

const VERSION: &str = "1.0.0";

lazy_static! {
    static ref REGISTRY: Registry = Registry::new();
    static ref IP_TX_BPS: GaugeVec = GaugeVec::new(
        Opts::new("network_ip_tx_bps", "TX bits per second per IP"),
        &["local_ip", "nic"]
    )
    .unwrap();
    static ref IP_RX_BPS: GaugeVec = GaugeVec::new(
        Opts::new("network_ip_rx_bps", "RX bits per second per IP"),
        &["local_ip", "nic"]
    )
    .unwrap();
    static ref TOTAL_TX_BPS: GaugeVec = GaugeVec::new(
        Opts::new(
            "network_ip_tx_bps_total",
            "Total TX bits per second per NIC"
        ),
        &["nic"]
    )
    .unwrap();
    static ref TOTAL_RX_BPS: GaugeVec = GaugeVec::new(
        Opts::new(
            "network_ip_rx_bps_total",
            "Total RX bits per second per NIC"
        ),
        &["nic"]
    )
    .unwrap();
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct NicConfig {
    lan: String,
    wan0: String,
    wan1: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StatusResponse {
    config: NicConfig,
    mappings: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct LocalSubnets {
    subnets: Vec<ipnet::Ipv4Net>,
}

impl LocalSubnets {
    fn new() -> Self {
        Self {
            subnets: Vec::new(),
        }
    }

    fn add_subnet(&mut self, subnet: &str) -> Result<(), Box<dyn std::error::Error>> {
        let net: ipnet::Ipv4Net = subnet.parse()?;
        self.subnets.push(net);
        Ok(())
    }

    fn is_local(&self, ip: &str) -> bool {
        if let Ok(addr) = ip.parse::<Ipv4Addr>() {
            for subnet in &self.subnets {
                if subnet.contains(&addr) {
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Debug, Clone)]
struct TrafficStats {
    tx_bytes: HashMap<String, u64>,     // key: "nic:ip"
    rx_bytes: HashMap<String, u64>,     // key: "nic:ip"
    nic_tx_total: HashMap<String, u64>, // key: nic
    nic_rx_total: HashMap<String, u64>, // key: nic
}

impl TrafficStats {
    fn new() -> Self {
        Self {
            tx_bytes: HashMap::new(),
            rx_bytes: HashMap::new(),
            nic_tx_total: HashMap::new(),
            nic_rx_total: HashMap::new(),
        }
    }

    fn reset(&mut self) {
        self.tx_bytes.clear();
        self.rx_bytes.clear();
        self.nic_tx_total.clear();
        self.nic_rx_total.clear();
    }
}

async fn fetch_nic_mappings() -> Result<StatusResponse, Box<dyn std::error::Error>> {
    let response = reqwest::get("http://localhost:32599/status").await?;
    let status: StatusResponse = response.json().await?;
    Ok(status)
}

fn get_nic_for_ip(ip: &str, status: &StatusResponse) -> String {
    // Check if IP is in mappings
    if let Some(wan) = status.mappings.get(ip) {
        // Convert wan name to nic name
        match wan.as_str() {
            "wan0" => status.config.wan0.clone(),
            "wan1" => status.config.wan1.clone(),
            _ => status.config.wan0.clone(),
        }
    } else {
        // Default to wan0
        status.config.wan0.clone()
    }
}

async fn metrics_handler() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

async fn update_metrics(stats: Arc<Mutex<TrafficStats>>, _status: Arc<Mutex<StatusResponse>>) {
    let mut interval = time::interval(Duration::from_secs(1));

    loop {
        interval.tick().await;

        let mut stats_guard = stats.lock().unwrap();

        // Update per-IP metrics
        for (key, &bytes) in &stats_guard.tx_bytes {
            let parts: Vec<&str> = key.split(':').collect();
            if parts.len() == 2 {
                let nic = parts[0];
                let ip = parts[1];
                let bps = (bytes * 8) as f64; // Convert bytes to bits
                IP_TX_BPS.with_label_values(&[ip, nic]).set(bps);
            }
        }

        for (key, &bytes) in &stats_guard.rx_bytes {
            let parts: Vec<&str> = key.split(':').collect();
            if parts.len() == 2 {
                let nic = parts[0];
                let ip = parts[1];
                let bps = (bytes * 8) as f64; // Convert bytes to bits
                IP_RX_BPS.with_label_values(&[ip, nic]).set(bps);
            }
        }

        // Update total metrics
        for (nic, &bytes) in &stats_guard.nic_tx_total {
            let bps = (bytes * 8) as f64;
            TOTAL_TX_BPS.with_label_values(&[nic]).set(bps);
        }

        for (nic, &bytes) in &stats_guard.nic_rx_total {
            let bps = (bytes * 8) as f64;
            TOTAL_RX_BPS.with_label_values(&[nic]).set(bps);
        }

        // Reset stats for next interval
        stats_guard.reset();
    }
}

fn capture_packets(
    interface_name: String,
    stats: Arc<Mutex<TrafficStats>>,
    status: Arc<Mutex<StatusResponse>>,
    local_subnets: Arc<LocalSubnets>,
) {
    tokio::task::spawn_blocking(move || {
        let device = Device::list()
            .expect("Failed to list devices")
            .into_iter()
            .find(|d| d.name == interface_name)
            .expect(&format!("Device {} not found", interface_name));

        let mut cap = Capture::from_device(device)
            .expect("Failed to open device")
            .promisc(true)
            .snaplen(65535)
            .timeout(1000)
            .open()
            .expect("Failed to activate capture");

        info!("Started capturing on {}", interface_name);

        loop {
            match cap.next_packet() {
                Ok(packet) => {
                    if let Some(ethernet) = EthernetPacket::new(packet.data) {
                        if ethernet.get_ethertype() == EtherTypes::Ipv4 {
                            if let Some(ipv4) = Ipv4Packet::new(ethernet.payload()) {
                                let src_ip = ipv4.get_source().to_string();
                                let dst_ip = ipv4.get_destination().to_string();
                                let packet_len = packet.data.len() as u64;

                                let status_guard = status.lock().unwrap();

                                // Determine if this is TX or RX based on source/destination
                                // TX: local IP is source
                                // RX: local IP is destination

                                // Check if source is local (TX)
                                if local_subnets.is_local(&src_ip) {
                                    let nic = get_nic_for_ip(&src_ip, &status_guard);
                                    let key = format!("{}:{}", nic, src_ip);
                                    let mut stats_guard = stats.lock().unwrap();
                                    *stats_guard.tx_bytes.entry(key).or_insert(0) += packet_len;
                                    *stats_guard.nic_tx_total.entry(nic).or_insert(0) += packet_len;
                                }

                                // Check if destination is local (RX)
                                if local_subnets.is_local(&dst_ip) {
                                    let nic = get_nic_for_ip(&dst_ip, &status_guard);
                                    let key = format!("{}:{}", nic, dst_ip);
                                    let mut stats_guard = stats.lock().unwrap();
                                    *stats_guard.rx_bytes.entry(key).or_insert(0) += packet_len;
                                    *stats_guard.nic_rx_total.entry(nic).or_insert(0) += packet_len;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    if !e.to_string().contains("timeout") {
                        error!("Error capturing packet: {}", e);
                    }
                }
            }
        }
    });
}

async fn refresh_mappings(status: Arc<Mutex<StatusResponse>>) {
    let mut interval = time::interval(Duration::from_secs(10));

    loop {
        interval.tick().await;
        match fetch_nic_mappings().await {
            Ok(new_status) => {
                let mut status_guard = status.lock().unwrap();
                *status_guard = new_status;
                info!("Updated NIC mappings");
            }
            Err(e) => {
                error!("Failed to fetch NIC mappings: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Parse local subnets from constant
    let mut local_subnets_obj = LocalSubnets::new();

    for subnet in LOCAL_SUBNETS {
        match local_subnets_obj.add_subnet(subnet) {
            Ok(_) => info!("Added local subnet: {}", subnet),
            Err(e) => error!("Failed to parse subnet '{}': {}", subnet, e),
        }
    }

    let local_subnets = Arc::new(local_subnets_obj);

    // Register metrics
    REGISTRY
        .register(Box::new(IP_TX_BPS.clone()))
        .expect("Failed to register IP_TX_BPS");
    REGISTRY
        .register(Box::new(IP_RX_BPS.clone()))
        .expect("Failed to register IP_RX_BPS");
    REGISTRY
        .register(Box::new(TOTAL_TX_BPS.clone()))
        .expect("Failed to register TOTAL_TX_BPS");
    REGISTRY
        .register(Box::new(TOTAL_RX_BPS.clone()))
        .expect("Failed to register TOTAL_RX_BPS");

    // Fetch initial NIC mappings
    let initial_status = match fetch_nic_mappings().await {
        Ok(status) => {
            info!("Fetched NIC mappings: {:?}", status);
            status
        }
        Err(e) => {
            error!("Failed to fetch initial NIC mappings: {}", e);
            error!("Using default configuration");
            StatusResponse {
                config: NicConfig {
                    lan: "eth2".to_string(),
                    wan0: "eth0".to_string(),
                    wan1: "eth1".to_string(),
                },
                mappings: HashMap::new(),
            }
        }
    };

    let stats = Arc::new(Mutex::new(TrafficStats::new()));
    let status = Arc::new(Mutex::new(initial_status.clone()));

    // Start packet capture
    let capture_interface = initial_status.config.lan.clone();
    capture_packets(
        capture_interface,
        stats.clone(),
        status.clone(),
        local_subnets.clone(),
    );

    // Start metrics updater
    let stats_clone = stats.clone();
    let status_clone = status.clone();
    tokio::spawn(async move {
        update_metrics(stats_clone, status_clone).await;
    });

    // Start periodic mappings refresh
    let status_clone = status.clone();
    tokio::spawn(async move {
        refresh_mappings(status_clone).await;
    });

    // Start HTTP server
    let app = Router::new().route("/metrics", get(metrics_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:59122")
        .await
        .unwrap();

    info!("version: {}", VERSION);

    info!("Prometheus metrics server listening on http://0.0.0.0:59122/metrics");

    axum::serve(listener, app).await.unwrap();
}
