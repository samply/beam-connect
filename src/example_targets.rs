use hyper::http::uri::Authority;
use shared::beam_id::{AppId, BeamId, BrokerId, ProxyId};

use crate::config::{CentralMapping, LocalMapping, LocalMappingEntry};

pub(crate) fn example_local(broker_id: &BrokerId) -> LocalMapping {
    let proxy23 = ProxyId::new(&format!("proxy23.{}", broker_id)).unwrap();
    let app1_id = AppId::new(&format!("connect1.{}",proxy23)).unwrap();
    let app2_id = AppId::new(&format!("connect2.{}",proxy23)).unwrap();
    let map = LocalMapping {entries: [
        ("ifconfig.me", "ifconfig.me", vec![app1_id.clone(), app2_id.clone()]),
        ("ip-api.com", "ip-api.com", vec![app1_id.clone(), app2_id.clone()]),
        ("wttr.in", "wttr.in", vec![app1_id.clone(), app2_id.clone()]),
        ("node23.uk12.network", "host23.internal.network", vec![app1_id, app2_id])
    ].map(|(needle,replace,allowed)| LocalMappingEntry {
        needle: Authority::from_static(needle),
        replace: Authority::from_static(replace),
        allowed
    })
    .into_iter()
    .collect()};
    map
}
