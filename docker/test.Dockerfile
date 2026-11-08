# Test image: server + CLI + tc, used to check the tool against simulated bad networks.
FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p server -p nt-cli

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends iproute2 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/server /usr/local/bin/nt-server
COPY --from=build /src/target/release/nt-cli /usr/local/bin/nt-cli
