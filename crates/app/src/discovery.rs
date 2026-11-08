//! Finding the other instances on the network (P29).
//!
//! An instance whose LAN remote is serving announces itself over mDNS, and an
//! instance that wants to read along browses for those announcements. Nobody
//! reads an address off one screen and types it into another — and when the
//! server's laptop is given a new lease halfway through a rehearsal, the
//! subscriber finds it again by its id rather than losing it.
//!
//! What is announced is deliberately little: a name an operator recognises
//! (the machine's), the id that stays the same across restarts and addresses,
//! and the version. Whether a key is needed is not — that is asked at the
//! door, by the client that wants in, like a browser does.
//!
//! **Discovery is a convenience, never a requirement.** Multicast does not
//! cross a VLAN boundary and some house networks filter it outright, so an
//! instance can always be added by address instead — see
//! [`crate::sources::probe`]. Nothing here is on the path a transcript takes.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use tauri::{AppHandle, Runtime};

/// The service type every instance announces itself under.
pub const SERVICE: &str = "_fluestr._tcp.local.";

/// Sent whenever the list of instances found on the network changes.
pub const EVENT_SERVERS: &str = "servers-found";

/// Another instance, as discovery or a probe found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "sources.ts")]
pub struct FoundServer {
    /// Its instance id — see [`crate::remote::instance_id`].
    pub id: String,
    /// The machine's name, which is what an operator recognises.
    pub name: String,
    pub address: String,
    pub port: u16,
    /// The app version it runs, when it said.
    pub version: Option<String>,
}

/// mDNS, both directions, behind one daemon started on first use.
///
/// Started lazily because the first multicast socket is what makes macOS ask
/// for the local network permission, and a machine that neither serves nor
/// reads along should never be asked.
#[derive(Default)]
pub struct Discovery {
    daemon: Mutex<Option<ServiceDaemon>>,
    found: Arc<Mutex<BTreeMap<String, FoundServer>>>,
    browsing: AtomicBool,
    /// This instance's own id, so that it does not list itself.
    own: Mutex<String>,
}

/// An announcement. Withdrawn when dropped, which is when the listener it
/// announces stops.
pub struct Advert {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Advert {
    fn drop(&mut self) {
        if let Err(err) = self.daemon.unregister(&self.fullname) {
            tracing::warn!(error = %err, "the announcement could not be withdrawn");
        }
    }
}

/// The machine's name as an operator would recognise it: `Studio-Mac` rather
/// than `Studio-Mac.local`.
pub fn machine_name() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    let name = name.trim_end_matches(".local").trim();
    if name.is_empty() {
        "Fluestr".to_string()
    } else {
        name.to_string()
    }
}

impl Discovery {
    pub fn set_own_id(&self, id: &str) {
        if let Ok(mut own) = self.own.lock() {
            *own = id.to_string();
        }
    }

    fn daemon(&self) -> Option<ServiceDaemon> {
        let mut slot = self.daemon.lock().ok()?;
        if slot.is_none() {
            match ServiceDaemon::new() {
                Ok(daemon) => *slot = Some(daemon),
                Err(err) => {
                    tracing::error!(error = %err, "mDNS could not be started");
                    return None;
                }
            }
        }
        slot.clone()
    }

    /// Announces a listener on `ip:port`.
    ///
    /// `None` for a loopback listener — nobody else can reach it, so telling
    /// the network about it would only put an entry in everybody's list that
    /// does not work — and when mDNS cannot be started, which costs the
    /// convenience and nothing else.
    pub fn advertise(&self, id: &str, ip: IpAddr, port: u16) -> Option<Advert> {
        if ip.is_loopback() {
            return None;
        }
        let daemon = self.daemon()?;
        let name = machine_name();
        // The id in the instance name, so two instances on one machine — or
        // two machines that were given the same name — never collide.
        let instance = format!("{name} ({})", &id[..id.len().min(6)]);
        let host = format!(
            "{}.local.",
            name.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect::<String>()
        );
        let properties = [
            ("id", id.to_string()),
            ("name", name.clone()),
            ("version", env!("CARGO_PKG_VERSION").to_string()),
        ];
        let info = match ServiceInfo::new(SERVICE, &instance, &host, ip, port, &properties[..]) {
            Ok(info) => info,
            Err(err) => {
                tracing::error!(error = %err, "the announcement could not be built");
                return None;
            }
        };
        let fullname = info.get_fullname().to_string();
        match daemon.register(info) {
            Ok(()) => {
                tracing::info!(%ip, port, name, "announced on the network");
                Some(Advert { daemon, fullname })
            }
            Err(err) => {
                tracing::error!(error = %err, "the announcement could not be registered");
                None
            }
        }
    }

    /// Starts looking for other instances, once per process.
    ///
    /// Runs until the app quits: a subscriber relies on it to follow an
    /// instance to a new address, and a list that is being looked at has to
    /// fill in as instances appear.
    pub fn browse<R: Runtime>(&self, app: &AppHandle<R>) {
        if self.browsing.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(daemon) = self.daemon() else {
            self.browsing.store(false, Ordering::SeqCst);
            return;
        };
        let events = match daemon.browse(SERVICE) {
            Ok(events) => events,
            Err(err) => {
                tracing::error!(error = %err, "browsing for other instances failed");
                self.browsing.store(false, Ordering::SeqCst);
                return;
            }
        };

        let found = Arc::clone(&self.found);
        let app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("discovery".into())
            .spawn(move || {
                // By full name, because that is what a removal names.
                let mut by_name: BTreeMap<String, String> = BTreeMap::new();
                while let Ok(event) = events.recv() {
                    let changed = match event {
                        ServiceEvent::ServiceResolved(service) => {
                            let id = service
                                .get_property_val_str("id")
                                .unwrap_or_default()
                                .to_string();
                            let Some(address) = best_address(&service.get_addresses_v4()) else {
                                continue;
                            };
                            if id.is_empty() {
                                continue;
                            }
                            let server = FoundServer {
                                id: id.clone(),
                                name: service
                                    .get_property_val_str("name")
                                    .unwrap_or(&service.fullname)
                                    .to_string(),
                                address: address.to_string(),
                                port: service.port,
                                version: service
                                    .get_property_val_str("version")
                                    .map(str::to_string),
                            };
                            by_name.insert(service.fullname.clone(), id.clone());
                            let Ok(mut found) = found.lock() else { break };
                            let same = found.get(&id) == Some(&server);
                            found.insert(id, server);
                            !same
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            match by_name.remove(&fullname) {
                                Some(id) => found
                                    .lock()
                                    .map(|mut f| f.remove(&id).is_some())
                                    .unwrap_or(false),
                                None => false,
                            }
                        }
                        _ => false,
                    };
                    if changed {
                        if let Some(discovery) = state_of(&app) {
                            crate::broadcast(&app, EVENT_SERVERS, discovery.servers());
                            crate::sources::moved(&app);
                        }
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::error!(error = %err, "the discovery thread could not be started");
            self.browsing.store(false, Ordering::SeqCst);
        }
    }

    /// Every other instance found, by name.
    pub fn servers(&self) -> Vec<FoundServer> {
        let own = self.own.lock().map(|o| o.clone()).unwrap_or_default();
        let mut servers: Vec<FoundServer> = self
            .found
            .lock()
            .map(|f| f.values().filter(|s| s.id != own).cloned().collect())
            .unwrap_or_default();
        servers.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        servers
    }

    /// Where the instance with this id is now, if it has been seen.
    pub fn find(&self, id: &str) -> Option<FoundServer> {
        self.found.lock().ok()?.get(id).cloned()
    }
}

fn state_of<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<Discovery>> {
    use tauri::Manager;
    app.try_state::<crate::AppState>()
        .map(|state| Arc::clone(&state.discovery))
}

/// The address to reach an instance on: a routable one before a link-local
/// one, which only works on the same segment and is what a machine falls back
/// to when DHCP did not answer.
fn best_address(addresses: &std::collections::HashSet<Ipv4Addr>) -> Option<Ipv4Addr> {
    let mut sorted: Vec<Ipv4Addr> = addresses
        .iter()
        .copied()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect();
    sorted.sort_by_key(|ip| (ip.is_link_local(), *ip));
    sorted.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routable_address_is_preferred_to_a_link_local_one() {
        let addresses = [
            Ipv4Addr::new(169, 254, 3, 4),
            Ipv4Addr::new(192, 168, 1, 20),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            best_address(&addresses),
            Some(Ipv4Addr::new(192, 168, 1, 20))
        );
    }

    #[test]
    fn nothing_usable_is_nothing() {
        let addresses = [Ipv4Addr::LOCALHOST].into_iter().collect();
        assert_eq!(best_address(&addresses), None);
    }
}
