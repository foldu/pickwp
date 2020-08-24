{ naersk
, protobuf
, sqlite
, openssl
, pkg-config
, perl
, installShellFiles
, test ? false
}:

let
  src = builtins.filterSource
    (path: type: type != "directory" || builtins.baseNameOf path != "target")
    ./.;
in
naersk.buildPackage {
  inherit src;
  singleStep = true;
  release = !test;
  doCheck = test;
  nativeBuildInputs = [
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
  PROTOC = "${protobuf}/bin/protoc";
  postInstall = ''
    installShellCompletion target/release/build/pickwp-*/out/pickwp.{fish,bash}
    installShellCompletion --zsh target/release/build/pickwp-*/out/_pickwp
  '';
}
