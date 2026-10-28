use beam_lib::{AppId, ProxyId};
use hyper::http::uri::Authority;

use crate::config::{AllowListEntry, LocalMapping, LocalMappingEntry};

pub(crate) fn example_local(broker_id: &str) -> LocalMapping {
    let proxy23 = ProxyId::new(&format!("proxy23.{}", broker_id)).unwrap();
    let app1_id =
        AllowListEntry::AppOrProxyId(AppId::new_unchecked(format!("connect1.{proxy23}")).into());
    let app2_id =
        AllowListEntry::AppOrProxyId(AppId::new_unchecked(format!("connect2.{proxy23}")).into());
    let map = LocalMapping {
        entries: [
            (
                "ifconfig.me",
                "ifconfig.me/asdf",
                vec![app1_id.clone(), app2_id.clone()],
            ),
            (
                "ip-api.com",
                "ip-api.com",
                vec![app1_id.clone(), app2_id.clone()],
            ),
            ("wttr.in", "wttr.in", vec![app1_id.clone(), app2_id.clone()]),
            (
                "node23.uk12.network",
                "host23.internal.network",
                vec![AllowListEntry::AppOrProxyId(proxy23.into())],
            ),
        ]
        .map(|(needle, replace, allowed)| LocalMappingEntry {
            needle: Authority::from_static(needle),
            replace: serde_json::from_value(serde_json::Value::String(replace.to_owned())).unwrap(),
            allowed,
            force_https: false,
            reset_host: false,
            external_path: None,
        })
        .into_iter()
        .collect(),
    };
    map
}
