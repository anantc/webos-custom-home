FROM rust:trixie
RUN apt-get update -qq && apt-get install -y --no-install-recommends \
    cmake clang libfontconfig-dev libfreetype-dev libx11-dev libxext-dev \
    libxrandr-dev libxcursor-dev libxi-dev libxfixes-dev libxss-dev libxtst-dev \
    libgl1-mesa-dev libegl1-mesa-dev libgl1-mesa-dri xvfb x11vnc novnc \
    python3 nodejs npm node-ws binutils xdotool \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add armv7-unknown-linux-gnueabi && rustup component add clippy rustfmt
RUN mkdir -p /opt/webos && curl -fL --retry 3 \
    https://github.com/webosbrew/native-toolchain/releases/latest/download/arm-webos-linux-gnueabi_sdk-buildroot_linux-aarch64.tar.bz2 \
    | tar xj -C /opt/webos \
    && sh /opt/webos/arm-webos-linux-gnueabi_sdk-buildroot/relocate-sdk.sh
RUN mkdir -p /opt/webos/SDL3-webos && curl -fL --retry 3 \
    https://github.com/webosbrew/SDL-webOS/releases/download/release-3.4.16-webos.5/SDL3-3.4.16-webos.5.tar.gz \
    | tar xz -C /opt/webos/SDL3-webos
RUN npm install -g @webos-tools/cli
ENV SDL3_PREFIX=/opt/webos/SDL3-webos
ENV TVHOME_WEBOS_NDK=/opt/webos/arm-webos-linux-gnueabi_sdk-buildroot
ENV SKIA_BINARIES_URL=https://git.unom.io/unom/skia-binaries/releases/download/0.153.3/skia-binaries-{key}.tar.gz
WORKDIR /workspace/native
