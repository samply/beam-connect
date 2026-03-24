# This assumes binaries are present, see COPY directive.

ARG IMGNAME=gcr.io/distroless/cc

FROM alpine AS chmodder
ARG FEATURE
ARG TARGETARCH
COPY /artifacts/binaries-$TARGETARCH$FEATURE/beam-connect /app/
RUN chmod +x /app/*

FROM ubuntu:latest
RUN apt update
RUN apt install -y ca-certificates ssl-cert

RUN make-ssl-cert generate-default-snakeoil
ENV TLS_TERMINATION_CERT_PATH=/etc/ssl/certs/ssl-cert-snakeoil.pem
ENV TLS_TERMINATION_KEY_PATH=/etc/ssl/private/ssl-cert-snakeoil.key

COPY --from=chmodder /app/* /usr/local/bin/
ENTRYPOINT [ "/usr/local/bin/beam-connect" ]
