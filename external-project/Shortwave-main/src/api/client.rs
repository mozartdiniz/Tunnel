// Shortwave - client.rs
// Copyright (C) 2021-2025  Felix Häcker <haeckerfelix@gnome.org>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::net::IpAddr;
use std::sync::Arc;

use gtk::gio;
use hickory_resolver::Resolver;
use hickory_resolver::config::ResolverConfig;
use hickory_resolver::name_server::TokioConnectionProvider;
use indexmap::IndexMap;
use rand::prelude::SliceRandom;
use rand::rng;
use reqwest::{Method, Request, StatusCode};
use serde::de;
use url::Url;

use crate::api::*;
use crate::app::SwApplication;
use crate::settings::{Key, settings_manager};

pub async fn station_request(
    request: StationRequest,
) -> Result<IndexMap<String, SwStation>, Error> {
    let url: Url = build_url(STATION_SEARCH, Some(&request.url_encode()))?;
    let stations_md: Vec<StationMetadata> = send_request(Request::new(Method::GET, url)).await?;

    // Creating hundreds of objects in a row can be expensive -> do it on separate thread
    let handle = gio::spawn_blocking(move || {
        let mut map = IndexMap::new();
        for station_md in stations_md {
            let uuid = station_md.stationuuid.clone();
            let station = SwStation::new(&uuid, false, station_md, None);
            map.insert(uuid, station);
        }
        map
    });
    let map = handle.await.unwrap();

    Ok(map)
}

pub async fn station_metadata_by_uuid(uuids: Vec<String>) -> Result<Vec<StationMetadata>, Error> {
    let url = build_url(STATION_BY_UUID, None)?;
    let uuids = format!(
        r#"{{"uuids":{}}}"#,
        serde_json::to_string(&uuids).unwrap_or_default()
    );
    debug!("Post body: {uuids}");

    let mut request = Request::new(Method::POST, url);
    *request.body_mut() = Some(uuids.into());

    let stations_md = send_request(request).await?;
    Ok(stations_md)
}

async fn send_request<T: de::DeserializeOwned + std::marker::Send + 'static>(
    mut request: Request,
) -> Result<T, Error> {
    request
        .headers_mut()
        .insert("Content-Type", "application/json".parse().unwrap());

    let response = crate::api::http::send(request).await.map_err(Arc::new)?;
    let status = response.status();
    let text = response.text().await.map_err(Arc::new)?;

    if status != StatusCode::OK {
        return Err(Error::InvalidHttpStatus(text));
    }

    // Deserializing JSON can be expensive -> do it on separate thread pool
    let to_deserialize = text.clone();
    let handle = gio::spawn_blocking(move || serde_json::from_str::<T>(&to_deserialize));
    let deserialized = handle.await.unwrap();

    match deserialized {
        Ok(d) => Ok(d),
        Err(err) => {
            error!("Unable to deserialize data: {err}, body: {text}");
            Err(Error::Deserializer(err.into()))
        }
    }
}

pub async fn lookup_rb_server() -> Option<String> {
    let lookup_domain = settings_manager::string(Key::ApiLookupDomain);
    let resolver = if let Ok(resolver) = Resolver::builder_tokio() {
        resolver.build()
    } else {
        warn!("Unable to use dns resolver from system conf");

        Resolver::builder_with_config(
            ResolverConfig::default(),
            TokioConnectionProvider::default(),
        )
        .build()
    };

    // Do forward lookup to receive a list with the api servers
    let response = resolver.lookup_ip(lookup_domain).await.ok()?;
    let mut ips: Vec<IpAddr> = response.iter().collect();

    // Shuffle it to make sure we're not using always the same one
    ips.shuffle(&mut rng());

    for ip in ips {
        // Do a reverse lookup to get the hostname
        let result = resolver
            .reverse_lookup(ip)
            .await
            .ok()
            .and_then(|r| r.into_iter().next());

        if result.is_none() {
            warn!("Reverse lookup for {ip} failed");
            continue;
        }

        // We need to strip the trailing "." from the domain name, otherwise TLS hostname verification fails
        let domain = result.unwrap().to_string();
        let hostname = domain.trim_end_matches(".");

        // Check if the server is online / returns data
        // If not, try using the next one in the list
        debug!("Trying to connect to {hostname} ({ip})");
        let url = Url::parse(&format!("https://{hostname}/{STATS}")).unwrap();
        let server_stats = send_request::<Stats>(Request::new(Method::GET, url)).await;

        match server_stats {
            Ok(stats) => {
                debug!(
                    "Successfully connected to {} ({}), server version {}, {} stations",
                    hostname, ip, stats.software_version, stats.stations
                );
                return Some(format!("https://{hostname}/"));
            }
            Err(err) => warn!("Unable to connect to {hostname}: {err}"),
        }
    }

    None
}

fn build_url(param: &str, options: Option<&str>) -> Result<Url, Error> {
    let rb_server = SwApplication::default().rb_server();
    if rb_server.is_none() {
        return Err(Error::NoServerAvailable);
    }

    let mut url = Url::parse(&rb_server.unwrap())
        .expect("Unable to parse server url")
        .join(param)
        .expect("Unable to join url");

    if let Some(options) = options {
        url.set_query(Some(options))
    }

    debug!("Retrieve data: {url}");
    Ok(url)
}
