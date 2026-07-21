{
  description = "Native Slint frontend for the Mullvad VPN daemon";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in {
      packages = forAllSystems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "mullvad-gui-slint";
            version = "0.1.0-alpha.1";
            src = self;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [
              libxkbcommon wayland libglvnd libdrm
              fontconfig freetype
              libx11 libxcursor libxi libxrandr libxext
            ];

            postInstall = ''
              DESTDIR="$out" PREFIX="" SKIP_BINARY_INSTALL=1 \
                bash ./scripts/stage-linux.sh
            '';

            meta = with pkgs.lib; {
              description = "Native Slint frontend for the Mullvad VPN daemon";
              homepage = "https://github.com/Greenstorm5417/Mullvad-Gui-Slint";
              license = licenses.gpl3Plus;
              mainProgram = "mullvad-gui-slint";
              platforms = supportedSystems;
            };
          };
        });

      devShells = forAllSystems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              rustc
              rustfmt
              slint-lsp
            ];
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [
              libxkbcommon wayland libglvnd libdrm
              fontconfig freetype
              libx11 libxcursor libxi libxrandr libxext
            ];

            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
              libxkbcommon wayland libglvnd libdrm
              libx11 libxcursor libxi libxrandr libxext
            ]);

            RUST_BACKTRACE = "1";
          };
        });
    };
}
