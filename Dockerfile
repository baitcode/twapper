FROM rust:slim AS builder
WORKDIR /app

COPY Cargo.toml .
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

COPY src src
RUN touch src/main.rs
RUN cargo build --release

RUN strip /app/target/release/twapper
RUN chown 1000:root /app/target/release/twapper 

FROM rust:slim AS release
WORKDIR /app

COPY --from=builder /app/target/release/twapper .

ENV ADDRESS=0.0.0.0
ENV PORT=3000
EXPOSE 3000

USER 1000
CMD ["./twapper"]
