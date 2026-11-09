FROM fedora:45

# Argumentos para mapear tu usuario real de Fedora y no tener problemas de permisos
ARG UID=1000
ARG GID=1000
ARG USER=developer

# 1. Instalar dependencias del sistema requeridas por GPUI, Vulkan y herramientas de depuración
RUN dnf update -y && dnf install -y \
    @development-tools \
    gcc-c++ \
    rustup \
    vulkan-loader-devel \
    vulkan-tools \
    fontconfig-devel \
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

# 3. Inicializar el toolchain stable y rust-analyzer sin usar curl
RUN rustup-init -y --default-toolchain stable --component rust-analyzer,rust-src
ENV PATH="/home/${USER}/.cargo/bin:${PATH}"

# Mantener el contenedor vivo en segundo plano
CMD ["sleep", "infinity"]
