# This Dockerfile is composed of two steps: the first one builds the release
# binary, and then the binary is copied inside another, empty image.

#################
#  Build image  #
#################

FROM ubuntu:bionic AS build

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    --no-install-recommends \
    ca-certificates \
    curl \
    gcc g++ \
    pkg-config \
    libssl-dev

RUN curl https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init >/tmp/rustup-init && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y --no-modify-path --default-toolchain stable --profile minimal
ENV PATH=/root/.cargo/bin:$PATH

WORKDIR /tmp/source
COPY . /tmp/source/
RUN cargo build --release

##################
#  Output image  #
##################

FROM ubuntu:bionic AS binary

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    ca-certificates

COPY --from=build /tmp/source/target/release/triagebot /usr/local/bin/
ENV PORT=80
CMD triagebot
