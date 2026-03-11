FROM scratch AS minimal
ARG BINARY=target/x86_64-unknown-linux-musl/release/tty-web
COPY ${BINARY} /tty-web
ENTRYPOINT ["/tty-web"]

FROM --platform=$BUILDPLATFORM ubuntu:24.04@sha256:d1e2e92c075e5ca139d51a140fff46f84315c0fdce203eab2807c7e495eff4f9 AS playground

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8
ENV TERM=xterm-256color

RUN --mount=type=cache,target=/var/lib/apt/lists \
    apt install -y --update ca-certificates && \
    apt install -y --update --no-install-recommends \
    curl \
    git \
    htop \
    neofetch \
    unzip \
    vim \
    zsh

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/local/bin/
RUN uv python install 3.14 --compile-bytecode \
    && for bin in $(dirname $(uv python find 3.14))/*; do \
        ln -s "$bin" /usr/local/bin/$(basename "$bin"); \
    done

RUN curl -fsSL https://fnm.vercel.app/install | bash -s -- --skip-shell \
    && export PATH="/root/.local/share/fnm:${PATH}" \
    && eval "$(fnm env)" \
    && fnm install 22 \
    && ln -s $(fnm exec --using 22 which node) /usr/local/bin/node \
    && ln -s $(fnm exec --using 22 which npm) /usr/local/bin/npm \
    && ln -s $(fnm exec --using 22 which npx) /usr/local/bin/npx

ARG GO_VERSION=1.24.1
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-$(dpkg --print-architecture).tar.gz" \
    | tar -C /usr/local -xz
ENV PATH="/usr/local/go/bin:${PATH}"

ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH="/usr/local/cargo/bin:${PATH}"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal

ARG NVIM_VERSION=0.10.4
RUN ARCH=$(uname -m) && [ "$ARCH" = "aarch64" ] && ARCH=arm64; \
    curl -fsSL "https://github.com/neovim/neovim/releases/download/v${NVIM_VERSION}/nvim-linux-${ARCH}.tar.gz" \
    | tar -C /usr/local --strip-components=1 -xz

RUN git clone --depth 1 https://github.com/AstroNvim/template ~/.config/nvim \
    && nvim --headless "+Lazy! sync" +qa

ARG OHMYZSH_COMMIT=41c5b9677afaf239268197546cfc8e003a073c97
ARG OHMYZSH_SHA256=ce0b7c94aa04d8c7a8137e45fe5c4744e3947871f785fd58117c480c1bf49352
RUN curl -fsSL "https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/${OHMYZSH_COMMIT}/tools/install.sh" -o /tmp/install.sh \
    && echo "${OHMYZSH_SHA256}  /tmp/install.sh" | sha256sum -c - \
    && sh /tmp/install.sh \
    && rm /tmp/install.sh

ARG BINARY=target/x86_64-unknown-linux-musl/release/tty-web
COPY ${BINARY} /usr/local/bin/tty-web

EXPOSE 9090

CMD ["tty-web", "--address", "0.0.0.0", "--port", "9090", "--shell", "/bin/zsh"]
