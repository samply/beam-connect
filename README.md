![Logo](./doc/Logo.svg) <!-- TODO: New Logo -->

Samply.Beam.Connect is a HTTP-relaying application for the distributed task broker [Samply.Beam](https://github.com/samply/beam). It is used to provide a transparent HTTP encapsulation method to transfer requests and replies via the Samply.Beam task mechanism. Its usage follows usual HTTP proxy semantics.

## Usage
In this description, we assume that you already have a running Samply.Beam
installation consisting of a central Samply.Beam.Broker, a central
Samply.Beam.CA and at least two local Samply.Proxies. To set this up, please see
the [Samply.Beam Documentation](https://github.com/samply/beam/blob/main/README.md).

You can either build and run Samply.Beam.Connect as a local application, or use
the docker image provided at the [Docker Hub](https://hub.docker.com/r/samply/beam-connect).

Samply.Beam.Connect provides two not exclusive modes:
 2. The Forwarder Mode, in which Beam.Connect encapsulates a HTTP Request and
    sends it into the Samply.Beam Pipeline
 1. The Receiver Mode, in which Beam.Connect receives an encapsulated HTTP
    Request and replies by communication with the desired endpoint.
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
./connect --proxy-url <PROXY_URL> --app-id <APP_ID> (--proxy-apikey <PROXY_APIKEY>)
```
The following command line parameters are required:
 * `PROXY_URL`: The URL of the local Samply.Proxy which is used to connect to
   the Samply.Broker
 * `APP_ID`: The BeamId of the Beam.Connect application
The following command line parameter is only used in Receiver mode:
 * `PROXY_APIKEY`: In Receiver Mode, the API key with which this Beam.Connector is registered for listening at
   the Samply.Broker
All parameters can be given as environment variables instead.


### Run using Docker
To run Samply.Beam.Connect via docker, first pull the recent Beam.Connect docker image:
```bash
docker pull samply/beam-connect
```
Run the container while providing the same parameters as
[before](#run-as-an-application) as environment variables:
```bash
docker run -e PROXY_URL='<Proxy_url>' \
           -e APP_ID='<App_Id>' \
           -e PROXY_APIKEY='<API_Key>' \
           samply/beam-connect
```
Again, the last environment variable `PROXY_APIKEY` is only required for usage
in Receiver Mode.

### Use Beam.Connect to forward a HTTP request
We give an example [cURL](https://curl.se/) request showing the usage of
Beam.Connect:
```bash
curl -x http://localbeamconnect:8081 -H "Proxy-Authorization: ApiKey connect1.proxy23.localhost Connect1Secret" -H "Authorization: basic YWxhZGRpbjpvcGVuc2VzYW1l" http://ip-api.com/json
```

The parameter `-x http://localbeamconnect:8081` instructs cURL to use
Beam.Connect as a HTTP proxy.

The header field `-H "Proxy-Authorization: ApiKey
connect1.proxy23.localhost Connect1Secret"` is the authentication information
between Beam.Connect and Samply.Proxy.

The `-H "Authorization: basic
YWxhZGRpbjpvcGVuc2VzYW1l"` header will be forwarded unmodified to the intended
resource.

Finally, `http://ip.api.com/json` is the requested resource.

A configurable mapping between requested resource and Beam.AppId must be
provided for routing the messages.

A mishap in communication will be returned as appropriate HTTP replies.


## Notes
At the moment Samply.Beam.Connect does not implement streaming and does not
support HTTPS connection. In the intended usage scenario, both Beam.Connect and
Samply.Proxy are positioned in the same privileged network, thus, not incurring
any security risks. The Samply.Proxy signs and encrypts the payloads on its own.

In Receiving Mode, Beam.Connect only relays requests to allow-listed resources
to mitigate possible misuse.

