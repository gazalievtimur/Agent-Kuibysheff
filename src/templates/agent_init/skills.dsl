skill "workspace" {
  policy: "Read inputs and write deliverables only through the sandboxed home tools."
  allowed_tools: ["home.list", "home.read", "home.write"]
}
