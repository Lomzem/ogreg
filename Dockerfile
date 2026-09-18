# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.98.1
FROM rust:${RUST_VERSION}-slim-trixie AS toolchain

ARG TARGETARCH
RUN test "$TARGETARCH" = amd64 || \
    (echo "Build with --platform linux/amd64 to produce x86-64 binaries." >&2; exit 1)

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        musl-tools \
        gcc-mingw-w64-x86-64-win32 \
        binutils-mingw-w64-x86-64 \
        file \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl x86_64-pc-windows-gnu

ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-win32

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY tests/ ./tests/

FROM toolchain AS linux-build
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,id=register-cli-linux,sharing=locked \
    cargo test --locked --target x86_64-unknown-linux-musl \
    && cargo build --release --locked --target x86_64-unknown-linux-musl \
    && mkdir -p /out/linux-x86_64 \
    && cp target/x86_64-unknown-linux-musl/release/register-cli /out/linux-x86_64/ \
    && /out/linux-x86_64/register-cli --version \
    && file /out/linux-x86_64/register-cli \
    && file -b /out/linux-x86_64/register-cli | grep -Eq 'ELF 64-bit.*x86-64.*static'

FROM toolchain AS windows-build
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,id=register-cli-windows,sharing=locked \
    cargo build --release --locked --target x86_64-pc-windows-gnu \
    && mkdir -p /out/windows-x86_64 \
    && cp target/x86_64-pc-windows-gnu/release/register-cli.exe /out/windows-x86_64/ \
    && file /out/windows-x86_64/register-cli.exe \
    && x86_64-w64-mingw32-objdump -p /out/windows-x86_64/register-cli.exe > /out/imports.txt \
    && if grep -Eiq 'DLL Name: (libgcc|libstdc\+\+|libwinpthread)' /out/imports.txt; then \
        cat /out/imports.txt >&2; \
        echo "The executable requires an unexpected compiler runtime DLL." >&2; \
        exit 1; \
    fi

FROM scratch AS artifacts
COPY --from=linux-build /out/linux-x86_64/ /linux-x86_64/
COPY --from=windows-build /out/windows-x86_64/ /windows-x86_64/
