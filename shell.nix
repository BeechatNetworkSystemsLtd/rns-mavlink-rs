with import <nixpkgs> {};
mkShell {
  buildInputs = [
    gdb # required for rust-gdb
    protobuf
    (python3.withPackages (p: with p; [
      ipython
      python-lsp-server
      scapy
    ]))
    rustup
    rust-analyzer
  ];
}
