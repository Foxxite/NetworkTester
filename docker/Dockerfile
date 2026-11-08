FROM rust:1-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p server

FROM debian:stable-slim
COPY --from=build /src/target/release/server /usr/local/bin/nt-server
# Ports: 7001/udp (ping, jitter, UDP load) and 7002/tcp (TCP ping, speed, TCP load).
EXPOSE 7001/udp 7002/tcp
USER nobody
ENTRYPOINT ["nt-server"]
