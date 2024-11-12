FROM debian:bookworm AS aravisbuilder

RUN apt-get update && \ 
    apt-get install -y git build-essential libreadline-dev zlib1g-dev flex bison libxml2-dev libxslt-dev libssl-dev python3 pipx libxml2-utils xsltproc ccache pkg-config && \
    rm -rf /var/lib/apt/lists/*


RUN apt-get update 
RUN apt-get install -y libglib2.0-dev zlib1g-dev libusb-1.0-0-dev && \
    rm -rf /var/lib/apt/lists/*

ENV PIPX_HOME=/opt/pipx 
ENV PIPX_BIN_DIR=/usr/local/bin

# install meson for our incremental build
RUN pipx ensurepath
RUN pipx install meson
RUN pipx install ninja


RUN git clone https://github.com/AravisProject/aravis.git && \
    cd aravis && \
    git checkout ARAVIS_0_8_13 

RUN cd aravis && meson setup build && meson configure build -Dviewer=disabled -Dusb=disabled -Dgst-plugin=disabled && meson compile -C build

FROM rust:1.82-bookworm AS rustbuilder
WORKDIR /app
COPY Cargo.toml .
COPY src/ src/
COPY --from=aravisbuilder /aravis/build /aravis/build
ENV PKG_CONFIG_PATH=/aravis/build/meson-uninstalled
#RUN cargo build

COPY examples/simple/ examples/simple/
RUN cd examples/simple && cargo build

FROM debian:bookworm
ENV LD_LIBRARY_PATH=/aravis
COPY --from=aravisbuilder /aravis/build/src /aravis
COPY --from=rustbuilder /app/examples/simple/target/debug/simple /simple
RUN apt update && \
    apt install -y libglib2.0-0 zlib1g && \
    rm -rf /var/lib/apt/lists/*
ENTRYPOINT [ "./simple"] 

#RUN apt update && apt install -y git
#RUN mkdir /root/.ssh/ && echo '[git.5enso.ch]:2224 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOUoDZC0NZYH1ZgQPxSAb+6uBDKZZCeWtzHbHsCyIXMW' >> /root/.ssh/known_hosts
#RUN --mount=type=ssh git clone ssh://git@git.5enso.ch:2224/5enso/pilatus-tbm.git
