skill "workspace" {
  policy: "Investigate and edit only the SWE-bench task repository under /testbed via workspace MCP. Never use host home tools. Never request oracle patches or hidden tests."
  allowed_tools: [
    "workspace.read_file",
    "workspace.write_file",
    "workspace.search",
    "workspace.exec",
    "workspace.git_diff"
  ]
}
