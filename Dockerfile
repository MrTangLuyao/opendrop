# syntax=docker/dockerfile:1

# Build stage: static musl binary, built natively for the target arch.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src

# Compile dependencies first so that source edits reuse this layer.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
 && echo 'fn main() {}' > src/main.rs \
 && echo 'fn main() {}' > build.rs \
 && cargo build --release --locked \
 && rm -rf src build.rs

COPY build.rs ./
COPY src ./src
COPY web ./web
# touch: COPY keeps the context mtimes, which may be older than the dummy build.
RUN find build.rs src -type f -exec touch {} + \
 && cargo build --release --locked \
 && strip target/release/opendrop \
 && mkdir -p /out/data \
 && cp target/release/opendrop /out/opendrop \
 && chown 10001:10001 /out/data

# Final image: the binary and an empty data dir, nothing else.
FROM scratch
LABEL org.opencontainers.image.source="https://github.com/MrTangLuyao/opendrop" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.title="opendrop" \
      org.opencontainers.image.description="Self-hosted temporary file drop with 6 digit pickup codes"
COPY --from=build /out/opendrop /opendrop
COPY --from=build --chown=10001:10001 /out/data /data
WORKDIR /data
USER 10001:10001
ENV OPENDROP_DATA=/data \
    OPENDROP_ADDR=0.0.0.0:8080
EXPOSE 8080
VOLUME /data
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 CMD ["/opendrop", "health"]
ENTRYPOINT ["/opendrop"]
CMD ["serve"]
