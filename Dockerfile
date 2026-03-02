# This Dockerfile is composed of several steps, to make cargo chef work.
# The final step copies the triagebot binary inside another, empty image.

#################
#  Build image  #
#################

FROM rust:1.93 AS base

RUN cargo install --locked cargo-chef

RUN apt-get update -y && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      g++ \
      curl \
      ca-certificates \
      libc6-dev \
      make \
      libssl-dev \
      pkg-config \
      git \
      cmake \
      zlib1g-dev

FROM base AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM base AS builder

# Copy build recipe
COPY --from=planner /recipe.json recipe.json

# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json

# And now build the rest
COPY . .
RUN cargo build --release

##################
#  Output image  #
##################

FROM ubuntu:24.04 AS binary

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    ca-certificates

RUN mkdir -p /opt/triagebot

COPY --from=builder /target/release/triagebot /usr/local/bin/
COPY templates /opt/triagebot/templates
WORKDIR /opt/triagebot
ENV PORT=80
CMD triagebot
