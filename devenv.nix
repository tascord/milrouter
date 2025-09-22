{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

{
  env.GREET = "router";
  packages = [
    pkgs.git
    pkgs.openssl
  ];

  languages.rust = {
    enable = true;
    channel = "nightly";
  };

  scripts.rustupdate.exec = ''
    rustup toolchain install nightly
    rustup default stable
  '';

  enterShell = ''
    rustupdate
    git --version
  '';
  
}
