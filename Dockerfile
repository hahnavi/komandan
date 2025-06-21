FROM rustlang/rust:nightly-alpine AS builder

RUN apk add --no-cache \
    build-base \
    perl

WORKDIR /app

COPY . .

RUN cargo build --features vendored-openssl --release

FROM alpine:3.21

WORKDIR /app

COPY --from=builder /app/target/release/komandan /usr/local/bin/komandan

ENTRYPOINT ["komandan"]
