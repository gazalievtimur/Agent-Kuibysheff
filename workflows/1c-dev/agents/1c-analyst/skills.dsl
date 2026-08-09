skill "workspace" {
  policy: "Read in/ and write plan artifacts under out/ only. Optional home.run for read-only git/rg when configured."
  allowed_tools: ["home.list", "home.read", "home.write", "home.run"]
}

skill "platform_help" {
  policy: "Use 1C platform help MCP for BSL syntax, query language, and topics. Read-only."
  allowed_tools: [
    "sntx_sem.search_help",
    "sntx_sem.search_bsl_syntax",
    "sntx_sem.search_query_language",
    "sntx_sem.get_topic",
    "sntx_sem.find_examples",
    "sntx_sem.list_domains"
  ]
}

skill "conf_docs" {
  policy: "Search configuration documentation when conf-doc MCP is available. Read-only."
  allowed_tools: [
    "conf_doc.conf_doc_search",
    "conf_doc.conf_doc_get_object",
    "conf_doc.conf_doc_get_object_chunk",
    "conf_doc.conf_doc_query"
  ]
}

skill "code_index" {
  policy: "Search and navigate CF dump via code-index MCP. Read-only."
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

skill "local_research" {
  policy: "Fallback research over the product CF workspace via local_tools."
  allowed_tools: [
    "local_tools.search_docs",
    "local_tools.read_file"
  ]
}

skill "web_search" {
  policy: "Search the web and read public URLs through SearXNG. Read-only. Do not invent results. Web supplements the brief; it never replaces TZ."
  allowed_tools: [
    "searxng.searxng_web_search",
    "searxng.searxng_search_suggestions",
    "searxng.searxng_instance_info",
    "searxng.web_url_read"
  ]
}
