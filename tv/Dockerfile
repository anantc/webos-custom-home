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
COPY patches/sdl-compositor-cursor.patch /tmp/sdl-compositor-cursor.patch
RUN curl -fL --retry 3 \
    https://github.com/webosbrew/SDL-webOS/archive/f599b850f9b1347cebdbb9a806999ddd170ab661.tar.gz \
    -o /tmp/sdl-source.tar.gz \
    && echo "4eb53efca51bdf89319ca1b2461a61911abb063c60c7408e3ffed023e813f61d  /tmp/sdl-source.tar.gz" | sha256sum -c - \
    && mkdir -p /tmp/sdl-source \
    && tar xzf /tmp/sdl-source.tar.gz -C /tmp/sdl-source --strip-components=1 \
    && cd /tmp/sdl-source && patch -p1 < /tmp/sdl-compositor-cursor.patch \
    && cmake -S . -B build \
      -DCMAKE_TOOLCHAIN_FILE=/opt/webos/arm-webos-linux-gnueabi_sdk-buildroot/share/buildroot/toolchainfile.cmake \
      -DCMAKE_SYSROOT=/opt/webos/arm-webos-linux-gnueabi_sdk-buildroot/arm-webos-linux-gnueabi/sysroot \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/webos/SDL3-webos \
      -DWEBOS=ON -DSDL_SHARED=ON -DSDL_STATIC=OFF -DSDL_TESTS=OFF \
      -DSDL_OFFSCREEN=OFF -DSDL_DISKAUDIO=OFF -DSDL_KMSDRM=OFF \
      -DSDL_WEBOS_HAVE_LIBHELPER=ON -DSDL_VENDOR_INFO="webOS Home cursor patch" \
    && cmake --build build --parallel 4 \
    && cmake --install build \
    && rm -rf /tmp/sdl-source /tmp/sdl-source.tar.gz /tmp/sdl-compositor-cursor.patch
RUN npm install -g @webos-tools/cli
ENV SDL3_PREFIX=/opt/webos/SDL3-webos
ENV TVHOME_WEBOS_NDK=/opt/webos/arm-webos-linux-gnueabi_sdk-buildroot
ENV SKIA_BINARIES_URL=https://git.unom.io/unom/skia-binaries/releases/download/0.153.3/skia-binaries-{key}.tar.gz
WORKDIR /workspace/tv
