//! The bridge between routing engines and Ferrostar in high-level platform frameworks.
//!
//! This module provides:
//! - Important traits for framework implementers
//! - Generic HTTP request generation and response parsing for common routing APIs
//!
//! If you're reading this module documentation,
//! you are probably bringing Ferrostar to a new platform
//! where no high-level library like the native iOS and Android `FerrostarCore` exists.
//!
//! The first thing you should know is that
//! there isn't anything inherently special about the types at the root of this module.
//! These types are designed to be integral to higher-level platform libraries
//! (like those for iOS and Android) to ensure extensibility.
//! But they aren't used anywhere else in the crate.
//!
//! If you're implementing Ferrostar bindings for a new platform,
//! have a look at the [iOS implementation](https://github.com/stadiamaps/ferrostar/blob/main/apple/Sources/FerrostarCore/FerrostarCore.swift),
//! particularly how the initializer requires a [`RouteProvider`](https://github.com/stadiamaps/ferrostar/blob/main/apple/Sources/FerrostarCore/RouteProvider.swift).
//! All routing in the `FerrostarCore` class goes through this,
//! with the heavy lifting done by either a [`RouteAdapter`] or a custom provider
//! (native arbitrary code).
//! We suggest that other platforms follow a similar approach to maximize extensibility.
//!
//! If you're doing a direct integration calling this library directly,
//! you can probably go straight to the submodules for integrations with major routing engines
//! and then construct a [`NavigationController`](crate::navigation_controller::NavigationController)
//! with a route and configuration.
//! PRs are welcome for any routing API with an open specification (even if it's a commercial offering).
//!
//! Ferrostar also fully supports proprietary and on-device routing.
//! All you need to do is convert your routes into Ferrostar [Route]s,
//! and nothing in this module is strictly required to do that.

use crate::models::Waypoint;
use crate::models::{Route, UserLocation};
use crate::routing_adapters::error::InstantiationError;
use error::{ParsingError, RoutingRequestGenerationError};

#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::collections::BTreeMap as HashMap;
#[cfg(feature = "std")]
use std::{collections::HashMap, fmt::Debug};

#[cfg(feature = "wasm-bindgen")]
use serde_json::json;
#[cfg(feature = "wasm-bindgen")]
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

#[cfg(feature = "alloc")]
use alloc::{string::String, sync::Arc, vec::Vec};

use crate::routing_adapters::osrm::OsrmResponseParser;
use crate::routing_adapters::valhalla::ValhallaHttpRequestGenerator;

pub mod error;
pub mod osrm;
pub mod valhalla;

/// A route request generated by a [`RouteRequestGenerator`].
#[derive(PartialEq, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum RouteRequest {
    HttpPost {
        url: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    },
    HttpGet {
        url: String,
        headers: HashMap<String, String>,
    },
}

/// A trait describing any object capable of generating [`RouteRequest`]s.
///
/// The interface is intentionally generic. Every routing backend has its own set of
/// parameters, including a "profile," max travel speed, units of speed and distance, and more.
/// It is assumed that these properties will be set at construction time or otherwise configured
/// before use, so that we can keep the public interface as generic as possible.
///
/// Implementations may be either in Rust (most popular engines should eventually have Rust
/// glue code) or foreign code.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait RouteRequestGenerator: Send + Sync {
    /// Generates a routing backend request given the set of locations.
    ///
    /// While most implementations will treat the locations as an ordered sequence, this is not
    /// guaranteed (ex: an optimized router).
    // TODO: Arbitrary options; how can we make this generic???
    // TODO: Option for whether we should account for course over ground or heading.
    fn generate_request(
        &self,
        user_location: UserLocation,
        waypoints: Vec<Waypoint>,
    ) -> Result<RouteRequest, RoutingRequestGenerationError>;

    // TODO: "Trace attributes" request method? Maybe in a separate trait?
}

/// A generic interface describing any object capable of parsing a response from a routing
/// backend into one or more [`Route`]s.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait RouteResponseParser: Send + Sync {
    /// Parses a raw response from the routing backend into a route.
    ///
    /// We use a sequence of octets as a common interchange format.
    /// as this works for all currently conceivable formats (JSON, PBF, etc.).
    fn parse_response(&self, response: Vec<u8>) -> Result<Vec<Route>, ParsingError>;
}

/// The route adapter bridges between the common core and a routing backend where interaction takes place
/// over a generic request/response flow (typically over a network;
/// local/offline routers **do not use this object** as the interaction patterns are different).
///
/// This is essentially the composite of the [`RouteRequestGenerator`] and [`RouteResponseParser`]
/// traits, but it provides one further level of abstraction which is helpful to consumers.
/// As there is no way to signal compatibility between request generators and response parsers,
/// the [`RouteAdapter`] provides convenience constructors which take the guesswork out of it,
/// while still leaving consumers free to implement one or both halves.
///
/// In the future, we may provide additional methods or conveniences, and this
/// indirection leaves the design open to such changes without necessarily breaking source
/// compatibility.
/// One such possible extension would be the ability to fetch more detailed attributes in real time.
/// This is supported by the Valhalla stack, among others.
///
/// Ideas  welcome re: how to signal compatibility between request generators and response parsers.
/// I don't think we can do this in the type system, since one of the reasons for the split design
/// is modularity, including the possibility of user-provided implementations, and these will not
/// always be of a "known" type to the Rust side.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct RouteAdapter {
    request_generator: Arc<dyn RouteRequestGenerator>,
    response_parser: Arc<dyn RouteResponseParser>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl RouteAdapter {
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new(
        request_generator: Arc<dyn RouteRequestGenerator>,
        response_parser: Arc<dyn RouteResponseParser>,
    ) -> Self {
        Self {
            request_generator,
            response_parser,
        }
    }

    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new_valhalla_http(
        endpoint_url: String,
        profile: String,
        costing_options_json: Option<String>,
    ) -> Result<Self, InstantiationError> {
        let request_generator = Arc::new(ValhallaHttpRequestGenerator::with_costing_options_json(
            endpoint_url,
            profile,
            costing_options_json,
        )?);
        let response_parser = Arc::new(OsrmResponseParser::new(6));
        Ok(Self::new(request_generator, response_parser))
    }

    //
    // Proxied implementation methods.
    //

    pub fn generate_request(
        &self,
        user_location: UserLocation,
        waypoints: Vec<Waypoint>,
    ) -> Result<RouteRequest, RoutingRequestGenerationError> {
        self.request_generator
            .generate_request(user_location, waypoints)
    }

    pub fn parse_response(&self, response: Vec<u8>) -> Result<Vec<Route>, ParsingError> {
        self.response_parser.parse_response(response)
    }
}

/// JavaScript wrapper for `RouteAdapter`.
#[cfg(feature = "wasm-bindgen")]
#[wasm_bindgen(js_name = RouteAdapter)]
pub struct JsRouteAdapter(RouteAdapter);

#[cfg(feature = "wasm-bindgen")]
#[wasm_bindgen(js_class = RouteAdapter)]
impl JsRouteAdapter {
    /// Creates a new RouteAdapter with a Valhalla HTTP request generator and an OSRM response parser.
    /// At the moment, this is the only supported combination.
    #[wasm_bindgen(constructor)]
    pub fn new(
        endpoint_url: String,
        profile: String,
        costing_options_json: Option<String>,
    ) -> Result<JsRouteAdapter, JsValue> {
        RouteAdapter::new_valhalla_http(endpoint_url, profile, costing_options_json)
            .map(JsRouteAdapter)
            .map_err(|e| JsValue::from_str(&format!("{}", e)))
        // TODO: We should have a better error handling strategy here. Same for the other methods.
    }

    #[wasm_bindgen(js_name = generateRequest)]
    pub fn generate_request(
        &self,
        user_location: JsValue,
        waypoints: JsValue,
    ) -> Result<JsValue, JsValue> {
        let user_location: UserLocation = serde_wasm_bindgen::from_value(user_location)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        let waypoints: Vec<Waypoint> = serde_wasm_bindgen::from_value(waypoints)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        match self.0.generate_request(user_location, waypoints) {
            Ok(RouteRequest::HttpPost { url, headers, body }) => {
                serde_wasm_bindgen::to_value(&json!({
                    "method": "post",
                    "url": url,
                    "headers": headers,
                    "body": body,
                }))
                .map_err(|e| JsValue::from_str(&e.to_string()))
            }
            Ok(RouteRequest::HttpGet { url, headers }) => serde_wasm_bindgen::to_value(&json!({
                "method": "get",
                "url": url,
                "headers": headers,
            }))
            .map_err(|e| JsValue::from_str(&e.to_string())),
            Err(e) => Err(JsValue::from_str(&e.to_string())),
        }
    }

    #[wasm_bindgen(js_name = parseResponse)]
    pub fn parse_response(&self, response: Vec<u8>) -> Result<JsValue, JsValue> {
        match self.0.parse_response(response.into()) {
            Ok(routes) => serde_wasm_bindgen::to_value(&routes).map_err(JsValue::from),
            Err(error) => Err(JsValue::from_str(&error.to_string())),
        }
    }
}
