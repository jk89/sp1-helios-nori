# Use Alpine Linux as the base image for the build stage
FROM alpine:latest AS builder

# Install dependencies for building Rust applications
RUN apk add --no-cache \
    build-base \
    cargo \
    rustup \
    && rustup-init -y \
    && source $HOME/.cargo/env

# Set the working directory for the build stage
WORKDIR /usr/src/build

# Copy only the necessary files for building (Cargo.toml, Cargo.lock, and source code)
COPY . .

# Build the application
RUN cargo build --release --bin operator

# Final stage: Use Alpine Linux as the base image for the application runtime
FROM alpine:latest

# Set the working directory inside the container
WORKDIR /usr/src/app

# Copy the binary from the builder stage to the final image
COPY --from=builder /usr/src/build/target/release/operator .

# Optionally, clean up any unnecessary files if needed
RUN rm -rf /var/cache/apk/* /tmp/* /var/tmp/*

# Command to run the application
CMD ["./operator"]