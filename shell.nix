{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell rec {
  # 1. Добавляем пакеты для Vulkan и графики
  libs = with pkgs; [
    wayland
    wayland-protocols
    libxkbcommon
    libGL
    # Ключевые пакеты для Vulkan:
    vulkan-loader    # Загрузчик Vulkan (libvulkan.so)
    vulkan-tools     # Утилита vulkaninfo для проверки
    mesa             # Содержит драйверы RADV (AMD) и ANV (Intel)
    # Для видеокарт NVIDIA также может понадобиться:
    # (config.boot.kernelPackages.nvidiaPackages.stable.override { hardware.nvidia.open = false; }) 
  ];

  nativeBuildInputs = with pkgs; [
    rustc
    cargo
    rust-analyzer
    pkg-config
  ] ++ libs;

  # 2. Настраиваем LD_LIBRARY_PATH для библиотек
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath libs;

  # 3. Явно указываем Vulkan ICD-файлы (драйверы)
  # Это заставит wgpu искать драйверы в стандартном месте NixOS
  # и в пакете mesa внутри nix-shell.
  shellHook = ''
    export VK_ICD_FILENAMES="/run/opengl-driver/share/vulkan/icd.d:${pkgs.mesa}/share/vulkan/icd.d"
    echo "Vulkan ICDs: $VK_ICD_FILENAMES"
  '';
}
