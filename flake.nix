{
  description = "Native GTK4 frontend for the Mullvad VPN daemon";

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
            pname = "mullvad-gtk";
            version = "0.1.0-alpha.1";
            src = self;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgs; [
              pkg-config
              wrapGAppsHook4
            ];
            buildInputs = with pkgs; [ gtk4 ];

            postInstall = ''
              DESTDIR="$out" PREFIX="" BINARY_PATH=target/release/mullvad-gtk \
                ./scripts/stage-linux.sh
            '';

            meta = with pkgs.lib; {
              description = "Native GTK4 frontend for the Mullvad VPN daemon";
              homepage = "https://github.com/Greenstorm5417/Mullvad-GTK";
              license = licenses.gpl3Plus;
              mainProgram = "mullvad-gtk";
              platforms = supportedSystems;
            };
          };
        });
    };
}
