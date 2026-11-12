# One image for all three Cloud Run services; the role is the first argument:
#   kwil api | kwil worker | kwil blog
FROM rust:1-slim-bookworm AS build
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/kwil /usr/local/bin/kwil
ENV APP_ENV=production RUST_LOG=info
EXPOSE 8080
ENTRYPOINT ["kwil"]
CMD ["api"]
