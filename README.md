![Logo](./doc/Logo.svg) <!-- TODO: New Logo -->

Samply.Beam.Connect is a HTTP-relaying application for the distributed task broker [Samply.Beam](https://github.com/samply/beam). It is used to provide a transparent HTTP encapsulation method to transfer requests and replies via the Samply.Beam task mechanism. Its usage follows usual HTTP proxy semantics.

## Usage
In this description, we assume that you already have a running Samply.Beam installation consisting of a central Samply.Beam.Broker, a central Samply.Beam.CA and at least two local Samply.Proxies. To set this up, please see the [Samply.Beam Documentation](https://github.com/samply/beam/blob/main/README.md).

You can either build and run Samply.Beam.Connect as a local application, or use the docker image provided at the [Docker Hub](https://hub.docker.com/r/samply/beam-connect).

Samply.Beam.Connect provides two not exclusive modes:
 1. The Forwarder Mode, in which Beam.Connect encapsulates a HTTP Request and sends it into the Samply.Beam Pipeline
 2. The Receiver Mode, in which Beam.Connect receives an encapsulated HTTP Request and replies by communication with the desired endpoint.

Of course, one process of Beam.Connect can be used in both modes simultaneously.

### Download and build locally
Please clone this repository and change into the directory:
```bash
git clone https://github.com/samply/beam-connect.git
cd beam-connect
```
Start the rust build process via
```bash
cargo build --release
```

### Run as an application
Change into the binary folder (by default after building `./target/release`) and start
Samply.Beam.Connect with
```bash
./connect --proxy-url <PROXY_URL> --app-id <APP_ID> --local-targets-file <LOCAL_TARGETS_FILE> --discovery-url <DISCOVERY_URL> (--proxy-apikey <PROXY_APIKEY>) (--bind-addr 0.0.0.0:8062)
```
The following command line parameters are required:
 * `PROXY_URL`: The URL of the local Samply.Proxy which is used to connect to the Samply.Broker
 * `APP_ID`: The BeamId of the Beam.Connect application 
 * `LOCAL_TARGETS_FILE`: The path to the local service resolution file (see [Routing Section](#Request-Routing)).
 * `DISCOVERY_URL`: The URL (or local file) to be is queried to receive the central service discovery (see [Routing Section](#Request-Routing)).
 
The following command line parameter is only used in Receiver mode (see [Usage Section](#usage)):
 * `PROXY_APIKEY`: In Receiver Mode, the API key with which this Beam.Connector is registered for listening at the Samply.Broker
 
The following command line parameter is optional, as it uses a default value:
 * `BIND_ADDR`: The interface and port Beam.Connect is listening on. Defaults to `0.0.0.0:8062`.

If the following flag is optional.
 * `NO_AUTH`:  Samply.Beam.Connect does not require a `Proxy Authorization` header, i.e. it forwards requests without (client) authentication

All parameters can be given as environment variables instead.

### Run using Docker
To run Samply.Beam.Connect via docker, first pull the recent Beam.Connect docker image:
```bash
docker pull samply/beam-connect
```

Run the container while providing the same parameters as [before](#run-as-an-application) as environment variables:
```bash
docker run -e PROXY_URL='<PROXY_URL>' \
           -e APP_ID='<APP_ID>' \
           -e LOCAL_TARGETS_FILE='<LOCAL_TARGETS_FILE>' \
           -e DISCOVERY_URL='<DISCOVERY_URL>' \
           -e PROXY_APIKEY='<PROXY_APIKEY>' \
           -e BIND_ADDR='<BIND_ADDR>' \
           -e NO_AUTH='true' \
           samply/beam-connect
```
Again, the environment variable `PROXY_APIKEY` is only required for usage in Receiver Mode. `BIND_ADDR` and `NO_AUTH` are optional.

### Use Beam.Connect to forward a HTTP request
We give an example [cURL](https://curl.se/) request showing the usage of Beam.Connect to access an internal service within University Hospital #23 (`uk23`):
```bash
curl -x http://localbeamconnect:8081 -H "Proxy-Authorization: ApiKey connect1.proxy23.localhost Connect1Secret" -H "Authorization: basic YWxhZGRpbjpvcGVuc2VzYW1l" http://uk23.virtual/json
```

The parameter `-x http://localbeamconnect:8081` instructs cURL to use Beam.Connect as a HTTP proxy.

The header field `-H "Proxy-Authorization: ApiKey connect1.proxy23.localhost Connect1Secret"` is the authentication information
between Beam.Connect and Samply.Proxy.

The `-H "Authorization: basic YWxhZGRpbjpvcGVuc2VzYW1l"` header will be forwarded unmodified to the intended
resource.

Finally, `http://uk23.virtual/json` is the requested resource.

#### Request Routing
A configurable mapping between requested resource and Beam.AppId must be provided for routing the messages. This is done, first, with the central service discovery, that maps the HTTP authority part (`uk23.virtual` in the example) to the Beam.AppId via which it can be reached (Receiver). There, the same authority is subsequently replaced by an internal host name provided in the local targets file. Examples for the expected mappings are in the `examples/` folder.

A mishap in communication will be returned as appropriate HTTP replies.

If one or multiple reverse proxies are placed in front of Beam.Connect, the `Host` header might be overwritten by one of the proxies. In that case, you can set a `X-Replace-Host` header to manually replace the host header within Beam.Connect.

#### Site Discovery

As described in the [command line parameter list](#run-as-an-application), the central cite discovery is fetched from a given URL or local json file. However, to spare the local services from the need to express outward facing connections themselves, Samply.Beam.Connect exports this received information as a local REST endpoint: `GET http://<beam_connect_url>:<beam_connect_port>/sites`. Note, that the information is only fetched at startup and remains static for the program's lifetime.

#### HTTPS support

Https is supported but requires setting up the following parameters:
* `SSL_CERT_PEM`: Location to the pem file used for incoming SSL connections.
* `SSL_CERT_KEY`: Location to the corresponding key file for the SSL connections.
* `TLS_CA_CERTIFICATES_DIR`: May need to be set if the local target uses a self signed certificate which is not trusted by beam-connect. In this case the certificate of the target must be placed inside `TLS_CA_CERTIFICATES_DIR` as a pem file in order to be trusted.

## Notes
At the moment Samply.Beam.Connect does not implement streaming. In the intended usage scenario, both Samply.Beam.Connect and Samply.Beam.Proxy are positioned right next to each other in the same privileged network and thus speak plain HTTP or [HTTPS if configured](#https). Of course, for outgoing traffic, the Samply.Proxy signs and encrypts the payloads on its own.

In Receiving Mode, Beam.Connect only relays requests to allow-listed resources to mitigate possible misuse.
