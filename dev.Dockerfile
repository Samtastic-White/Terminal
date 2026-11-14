FROM fedora:45

# Argumentos para mapear tu usuario real de Fedora y no tener problemas de permisos
ARG UID=1000
ARG GID=1000
ARG USER=developer

# 1. Instalar dependencias del sistema requeridas por GPUI, Vulkan, fuentes y herramientas de depuración
RUN dnf update -y && dnf install -y \
    @development-tools \
    gcc-c++ \
    rustup \
    vulkan-loader-devel \
    vulkan-tools \
    fontconfig-devel \
    jetbrains-mono-fonts \
    google-noto-color-emoji-fonts \
    google-noto-sans-cjk-fonts \
    libxkbcommon-devel \
    libxkbcommon-x11-devel \
    wayland-devel \
    wayland-protocols-devel \
    libxcb-devel \
    git curl sudo zsh procps-ng && \
    dnf clean all

# 2. Configurar usuario no root idéntico a tu usuario de Fedora
RUN groupadd -g ${GID} ${USER} && \
    useradd -m -u ${UID} -g ${GID} -s /bin/zsh ${USER} && \
    echo "${USER} ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers

USER ${USER}
WORKDIR /workspace

# 3. Inicializar el toolchain stable con rust-analyzer, rust-src, clippy y rustfmt
ENV PATH="/home/${USER}/.cargo/bin:${PATH}"

RUN rustup-init -y --default-toolchain stable --profile default \
    --component rust-analyzer,rust-src,clippy,rustfmt && \
    rustc --version && cargo --version && rust-analyzer --version

# 4. Precargar el índice, los crates y la dependencia git de Zed en la imagen.
# El volumen cargo-cache hereda esta caché en el primer arranque, así que
# rust-analyzer no se queda en "Fetching: Cargo metadata".
COPY --chown=${UID}:${GID} Cargo.toml Cargo.lock /tmp/warm/
RUN cd /tmp/warm && \
    mkdir -p src && printf 'fn main() {}\n' > src/main.rs && \
    cargo fetch --locked && \
    rm -rf /tmp/warm

# Mantener el contenedor vivo en segundo plano
CMD ["sleep", "infinity"]
