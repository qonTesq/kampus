# syntax=docker/dockerfile:1
# kampus demo build (sandbox + broker). Build context: the repository root
# (see compose.yaml); Dockerfile.dockerignore limits what is sent.

# Same version as rust-toolchain.toml, which is copied in and wins if they drift.
FROM rust:1.99.0-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p kampus --features sandbox,broker \
 && cp target/release/kampus /kampus

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /kampus /usr/local/bin/kampus
WORKDIR /srv/kampus
USER nonroot
ENTRYPOINT ["/usr/local/bin/kampus"]
CMD ["serve", "--school", "data/school.yaml", "--policy", "data/policy.yaml"]
