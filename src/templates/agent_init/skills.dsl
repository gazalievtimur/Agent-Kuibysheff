skill "workspace" {
  policy: "Read inputs and write deliverables only through the sandboxed home tools."
  allowed_tools: ["home.list", "home.read", "home.write"]
}

skill "configure" {
  policy: "Change a saved agent profile only when the user asks. Edits apply on the next start. Do not read or write secrets."
  allowed_tools: [
    "config.show",
    "config.rules_get",
    "config.rules_set",
    "config.prompt_get",
    "config.prompt_set",
    "config.skill_list",
    "config.skill_get",
    "config.skill_add",
    "config.skill_set",
    "config.skill_remove",
    "config.skill_body_get",
    "config.skill_body_set",
    "config.skill_file_list",
    "config.skill_file_get",
    "config.skill_file_put",
    "config.skill_file_remove",
    "config.mcp_list",
    "config.mcp_get",
    "config.mcp_add",
    "config.mcp_set",
    "config.mcp_remove",
    "config.limits_get",
    "config.limits_set",
    "config.provider_get",
    "config.provider_set",
    "config.access_get",
    "config.builtins_add",
    "config.builtins_remove",
    "config.init"
  ]
}
