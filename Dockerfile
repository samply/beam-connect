# This assumes binaries are present, see COPY directive.

ARG IMGNAME=gcr.io/distroless/cc

FROM alpine AS chmodder
ARG FEATURE
ARG TARGETARCH
ARG COMPONENT=beam-connect
COPY /artifacts/binaries-$TARGETARCH$FEATURE/$COMPONENT /app/$COMPONENT
RUN chmod +x /app/*

# FROM ${IMGNAME}
FROM ubuntu:latest
RUN apt update
RUN apt install -y ca-certificates
RUN apt install -y ssl-cert
RUN make-ssl-cert generate-default-snakeoil
ARG COMPONENT=beam-connect
COPY --from=chmodder /app/$COMPONENT /usr/local/bin/samply
ENTRYPOINT [ "/usr/local/bin/samply" ]
