let
  pkgs = import <nixpkgs> {};
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    protobuf
  ];
  buildInputs = [
    pkgs.sqlite
  ];
  PROTOC = "${pkgs.protobuf}/bin/protoc";
}
