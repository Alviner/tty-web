FROM snakepacker/python:3.13

ADD packages.txt /usr/share/packages.txt
RUN xargs -ra /usr/share/packages.txt apt-install
ARG OHMYZSH_COMMIT=41c5b9677afaf239268197546cfc8e003a073c97
ARG OHMYZSH_SHA256=ce0b7c94aa04d8c7a8137e45fe5c4744e3947871f785fd58117c480c1bf49352
RUN curl -fsSL "https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/${OHMYZSH_COMMIT}/tools/install.sh" -o /tmp/install.sh \
    && echo "${OHMYZSH_SHA256}  /tmp/install.sh" | sha256sum -c - \
    && sh /tmp/install.sh \
    && rm /tmp/install.sh

COPY target/release/tty-web /usr/local/bin/tty-web
ENV LANG=C.UTF-8
ENV TERM=xterm-256color

EXPOSE 9090

CMD ["tty-web", "--address", "0.0.0.0", "--port", "9090", "--shell", "/bin/zsh"]
