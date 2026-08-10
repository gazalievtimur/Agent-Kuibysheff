skill "workspace" {
  policy: "Read, write, and run only through sandboxed home tools. Prefer python via home.run."
  allowed_tools: ["home.list", "home.read", "home.write", "home.run"]
}
