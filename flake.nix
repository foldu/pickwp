{
  description = "A thing.";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    naersk = {
      url = "github:nmattia/naersk";
      inputs.nixpkgs.follows = "/nixpkgs";
    };
  };

  outputs = { self, nixpkgs, naersk, flake-utils }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs { inherit system; };
      in
        {
          defaultPackage = naersk.lib.${system}.buildPackage {
            src = ./.;
            singleStep = true;
            nativeBuildInputs = with pkgs; [
              protobuf
              sqlite
              openssl
              pkg-config
              # openssl build
              perl
              installShellFiles
            ];
            preBuild = ''
              mkdir -p db
              sqlite3 db/db.sqlite -init ./sql/schema.sql .exit
            '';
            DATABASE_URL = "sqlite://db/db.sqlite";
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            postInstall = ''
              installShellCompletion target/release/build/pickwp-*/out/pickwp.{fish,bash}
              installShellCompletion --zsh target/release/build/pickwp-*/out/_pickwp
            '';
          };
          defaultApp = {
            type = "app";
            program = "${self.defaultPackage.${system}}/bin/pickwp";
          };
        }
    );
}
