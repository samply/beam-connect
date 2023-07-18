use hyper::http::uri::Authority;
use beam_lib::{AppId, ProxyId};

use crate::config::{LocalMapping, LocalMappingEntry};

pub(crate) fn example_local(broker_id: &str) -> LocalMapping {
    let proxy23 = ProxyId::new(&format!("proxy23.{}", broker_id)).unwrap();
    let app1_id = AppId::new(&format!("connect1.{}",proxy23)).unwrap();
    let app2_id = AppId::new(&format!("connect2.{}",proxy23)).unwrap();
    let map = LocalMapping {entries: [
        ("ifconfig.me", "ifconfig.me/asdf", vec![app1_id.clone(), app2_id.clone()]),
        ("ip-api.com", "ip-api.com", vec![app1_id.clone(), app2_id.clone()]),
        ("wttr.in", "wttr.in", vec![app1_id.clone(), app2_id.clone()]),
        ("node23.uk12.network", "host23.internal.network", vec![app1_id, app2_id])
    ].map(|(needle,replace,allowed)| LocalMappingEntry {
        needle: Authority::from_static(needle),
        replace: serde_json::from_value(serde_json::Value::String(replace.to_owned())).unwrap(),
        allowed
    })
    .into_iter()
    .collect()};
    map
}
