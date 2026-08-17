skill "workspace" {
  policy: "Read in/docs and plan; write tests under out/ only."
  allowed_tools: ["home.list", "home.read", "home.write", "home.run"]
}

skill "yaxunit_docs" {
  policy: "Public YAxUnit snapshot is under in/docs/. Read it before writing tests. Do not invent API."
  allowed_tools: ["home.list", "home.read", "local_tools.read_file", "local_tools.search_docs"]
}

skill "platform_help" {
  policy: "Platform help for BSL used inside tests (documents, queries). Read-only."
  allowed_tools: [
    "sntx_sem.search_help",
    "sntx_sem.search_bsl_syntax",
    "sntx_sem.get_topic",
    "sntx_sem.find_examples"
  ]
}

skill "code_index" {
  policy: "Read/search CF dump to know objects under test. Do not invent symbols."
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
  policy: "Supplement in/docs/ only. Public YAxUnit URLs. Cite them. Never invent results. Never use ITS."
  allowed_tools: [
    "searxng.searxng_web_search",
    "searxng.searxng_search_suggestions",
    "searxng.searxng_instance_info",
    "searxng.web_url_read"
  ]
}
