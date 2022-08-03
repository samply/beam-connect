use std::collections::HashMap;

use hyper::Uri;
use shared::beam_id::{AppId, BeamId};

use crate::structs::InternalHost;

pub fn get_examples() -> HashMap<InternalHost, AppId> {
    let app_id = AppId::new("connect1.proxy23.localhost").unwrap();
    let input = [
        ("http://ifconfig.me/", app1_id.clone()),
        ("http://ip-api.com/json", app1_id),
        ("http://wttr.in", app2_id)
    ].map(|(k,v)| (Uri::try_from(k).unwrap().authority().unwrap().to_owned(), v))
    .into_iter().collect();
    
    input
}
