skill "workspace" {
  policy: "Read plan inputs; write sources only under out/. home.run limited to read-only git/rg when configured."
  allowed_tools: ["home.list", "home.read", "home.write", "home.run"]
}

skill "platform_help" {
  policy: "Platform help for BSL/metadata questions. Read-only."
  allowed_tools: [
    "sntx_sem.search_help",
    "sntx_sem.search_bsl_syntax",
    "sntx_sem.get_topic",
    "sntx_sem.find_examples"
  ]
}

skill "code_index" {
  policy: "Read/search CF dump. Do not invent symbols."
  allowed_tools: [
    "code-index.search",
    "code-index.find_symbol",
    "code-index.get_file",
    "code-index.list_files",
    "code_index.search",
    "code_index.find_symbol",
    "code_index.get_file",
    "code_index.list_files"
  ]
}

skill "bsl_lint" {
  policy: "Analyze BSL when bsl-language-server MCP is available."
  allowed_tools: [
    "bsl-language-server.analyze",
    "bsl_language_server.analyze"
  ]
}

skill "local_research" {
  policy: "Read baseline CF files via local_tools."
  allowed_tools: [
    "local_tools.search_docs",
    "local_tools.read_file"
  ]
}
