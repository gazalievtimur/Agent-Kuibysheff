skill "workspace" {
  policy: "Read plan and coder inputs; write CFE tree under out/cfe/ only."
  allowed_tools: ["home.list", "home.read", "home.write", "home.run"]
}

skill "platform_help" {
  policy: "Minimal platform help when metadata structure is unclear."
  allowed_tools: [
    "sntx_sem.search_help",
    "sntx_sem.get_topic"
  ]
}

skill "code_index" {
  policy: "Verify against CF baseline. Read-only."
  allowed_tools: [
    "code-index.search",
    "code-index.get_file",
    "code-index.list_files",
    "code_index.search",
    "code_index.get_file",
    "code_index.list_files"
  ]
}

skill "local_research" {
  policy: "Read baseline CF for borrow/compare."
  allowed_tools: [
    "local_tools.search_docs",
    "local_tools.read_file"
  ]
}

skill "bsl_lint" {
  policy: "Analyze packed BSL under out/cfe when MCP available."
  allowed_tools: [
    "bsl-language-server.analyze",
    "bsl_language_server.analyze"
  ]
}
