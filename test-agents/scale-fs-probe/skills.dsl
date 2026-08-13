skill "workspace" {
  policy: "Search and read workspace corpus; read/write home; run python when files are too large for home.read."
  allowed_tools: [
    "home.list",
    "home.read",
    "home.write",
    "home.run",
    "local_tools.search_docs",
    "local_tools.read_file"
  ]
}
