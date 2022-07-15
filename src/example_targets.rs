use std::collections::HashMap;

use hyper::Uri;
use shared::beam_id::{AppId, BeamId};

use crate::structs::InternalHost;

pub fn get_examples() -> HashMap<InternalHost, AppId> {
    let app_id = AppId::new("pusher1.proxy23.localhost").unwrap();
    let input = [
        ("https://ifconfig.me/", app_id.clone()),
        ("http://ip-api.com/json", app_id)
    ].map(|(k,v)| (Uri::try_from(k).unwrap().authority().unwrap().to_owned(), v))
    .into_iter().collect();
    
    input
}