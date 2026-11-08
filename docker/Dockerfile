FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p server

FROM debian:stable-slim
COPY --from=build /src/target/release/server /usr/local/bin/nt-server
EXPOSE 7000/tcp 7001/udp 7002/tcp
ENTRYPOINT ["nt-server"]
