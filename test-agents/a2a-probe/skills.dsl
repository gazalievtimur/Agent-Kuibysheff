skill "workspace" {
  policy: "Read and write deliverables only through home filesystem tools under out/."
  allowed_tools: ["home.list", "home.read", "home.write"]
}
