skill "workspace" {
  policy: "Search and read workspace corpus; page large home files with home.read windows (next_offset); home.run only if the goal explicitly requires a script."
  allowed_tools: [
    "home.list",
    "home.read",
    "home.write",
    "home.run",
    "local_tools.search_docs",
    "local_tools.read_file"
  ]
}
