let
  pkgs = import <nixpkgs> {};
  db = "db/db.sqlite";
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    protobuf
  ];
  buildInputs = [
    pkgs.sqlite
    pkgs.openssl
    pkgs.pkg-config
  ];
  PROTOC = "${pkgs.protobuf}/bin/protoc";
  DATABASE_URL = "sqlite://${db}";
  shellHook = ''
    mkdir -p "$(dirname "${db}")"
    if ! test -f "${db}"; then
      sqlite3 "${db}" -init ./sql/schema.sql .exit
    fi
  '';
}
