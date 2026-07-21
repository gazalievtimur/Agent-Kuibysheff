skill "workspace" {
  policy: "Read inputs and write deliverables only through the sandboxed home tools."
  allowed_tools: ["home.list", "home.read", "home.write"]
}

skill "web_search" {
  policy: "Search the web and read public URLs through the configured SearXNG MCP. Read-only. Do not invent results."
  allowed_tools: [
    "searxng.searxng_web_search",
    "searxng.searxng_search_suggestions",
    "searxng.searxng_instance_info",
    "searxng.web_url_read"
  ]
}
