use async_trait::async_trait;
use feilian_ipc::{TransportProtocol, TunnelMode, TunnelSpec, TunnelStats};
use feilian_lite::{start_wg_go, start_wg_go_netstack, stop_wg_go, UAPIClient, WgConf};

use crate::TunnelBackend;

#[derive(Default)]
pub struct LibwgBackend {
    running: bool,
    interface_name: Option<String>,
}

#[async_trait]
impl TunnelBackend for LibwgBackend {
    async fn start(&mut self, spec: &TunnelSpec) -> Result<(), String> {
        if self.running {
            return Err("libwg is already running".to_string());
        }

        let config = wg_config(spec);
        let start_result = match &spec.mode {
            TunnelMode::SystemSplit => start_wg_go(&spec.interface_name, config.protocol, false),
            TunnelMode::Socks5 {
                listen_addr,
                username,
                password,
            } => start_wg_go_netstack(
                &config,
                listen_addr,
                username.as_deref().unwrap_or_default(),
                password.as_deref().unwrap_or_default(),
                false,
            ),
        };
        start_result.map_err(|error| error.to_string())?;

        let mut uapi = UAPIClient {
            name: spec.interface_name.clone(),
        };
        let configure_result = match spec.mode {
            TunnelMode::SystemSplit => uapi.config_wg(&config).await,
            TunnelMode::Socks5 { .. } => uapi.config_wg_netstack(&config).await,
        };
        if let Err(error) = configure_result {
            stop_wg_go();
            return Err(error.to_string());
        }

        self.running = true;
        self.interface_name = Some(spec.interface_name.clone());
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), String> {
        self.stop_if_running();
        Ok(())
    }

    async fn stats(&mut self) -> Result<TunnelStats, String> {
        if !self.running {
            return Ok(TunnelStats::default());
        }
        let name = self
            .interface_name
            .as_ref()
            .ok_or_else(|| "running libwg instance has no interface name".to_string())?;
        let stats = UAPIClient { name: name.clone() }
            .stats()
            .map_err(|error| error.to_string())?;
        Ok(TunnelStats {
            tx_bytes: stats.tx_bytes,
            rx_bytes: stats.rx_bytes,
        })
    }

    async fn cleanup(&mut self) -> Result<(), String> {
        self.stop_if_running();
        Ok(())
    }
}

impl LibwgBackend {
    fn stop_if_running(&mut self) {
        if self.running {
            stop_wg_go();
            self.running = false;
            self.interface_name = None;
        }
    }
}

impl Drop for LibwgBackend {
    fn drop(&mut self) {
        self.stop_if_running();
    }
}

fn wg_config(spec: &TunnelSpec) -> WgConf {
    WgConf {
        address: spec.address.clone(),
        address6: spec.address6.clone(),
        peer_address: spec.peer_address.clone(),
        mtu: spec.mtu,
        public_key: spec.public_key.clone(),
        private_key: spec.private_key.clone(),
        peer_key: spec.peer_key.clone(),
        allowed_ips: spec.allowed_ips.clone(),
        routes: spec.routes.clone(),
        dns: spec.dns.clone(),
        protocol: match spec.protocol {
            TransportProtocol::Udp => 0,
            TransportProtocol::Tcp => 1,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(protocol: TransportProtocol) -> TunnelSpec {
        TunnelSpec {
            node_name: "node-a".to_string(),
            interface_name: "feilian".to_string(),
            mode: TunnelMode::SystemSplit,
            address: "10.0.0.2/32".to_string(),
            address6: "fd00::2/128".to_string(),
            peer_address: "192.0.2.1:51820".to_string(),
            mtu: 1380,
            public_key: "public".to_string(),
            private_key: "private".to_string(),
            peer_key: "peer".to_string(),
            allowed_ips: vec!["10.0.0.0/8".to_string()],
            routes: vec!["10.0.0.0/8".to_string()],
            dns: "10.0.0.53".to_string(),
            protocol,
        }
    }

    #[test]
    fn maps_ipc_tunnel_spec_to_libwg_configuration() {
        let config = wg_config(&spec(TransportProtocol::Tcp));

        assert_eq!(config.address, "10.0.0.2/32");
        assert_eq!(config.address6, "fd00::2/128");
        assert_eq!(config.mtu, 1380);
        assert_eq!(config.protocol, 1);
        assert_eq!(config.allowed_ips, vec!["10.0.0.0/8"]);
    }
}
