# This Dockerfile is composed of two steps: the first one builds the release
# binary, and then the binary is copied inside another, empty image.

#################
#  Build image  #
#################

FROM rust:1.45 AS build

COPY . .
RUN cargo test --release --all
RUN cargo build --release

##################
#  Output image  #
##################

FROM ubuntu:bionic AS binary

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    ca-certificates

RUN mkdir -p /opt/triagebot

COPY --from=build /target/release/triagebot /usr/local/bin/
COPY templates /opt/triagebot/templates
WORKDIR /opt/triagebot
ENV PORT=80
CMD triagebot
