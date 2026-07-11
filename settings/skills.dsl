skill "workspace" {
  policy: "Read inputs and write deliverables only through the sandboxed home tools."
  allowed_tools: ["home.list", "home.read", "home.write"]
}

skill "research" {
  policy: "Use configured read-only MCP research tools when the task requires repository context."
  allowed_tools: ["local_tools.search_docs", "local_tools.read_file"]
}
