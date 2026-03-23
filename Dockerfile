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
ENV SSL_CERT_PEM=/etc/ssl/certs/ssl-cert-snakeoil.pem
ENV SSL_CERT_KEY=/etc/ssl/private/ssl-cert-snakeoil.key

COPY --from=chmodder /app/* /usr/local/bin/
ENTRYPOINT [ "/usr/local/bin/beam-connect" ]
