# Runtime image for the locally built Stellar backend.
#
# The binary embeds the Angular frontend (rust-embed), so it is built from
#   frontend/  ->  ng build --configuration production --base-href ./
#   backend/   ->  cargo build --release
# and the resulting binary is copied in here.
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false stellar

WORKDIR /app
RUN mkdir -p bin conf lib data logs

COPY backend/target/release/stellar ./bin/stellar
COPY backend/migrations ./migrations

RUN chown -R stellar:stellar /app && chmod +x ./bin/stellar

USER stellar

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["./bin/stellar"]
