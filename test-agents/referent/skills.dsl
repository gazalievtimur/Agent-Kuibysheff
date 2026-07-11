skill "workspace" {
  policy: "Read inputs and write research deliverables only through the sandboxed home tools."
  allowed_tools: ["home.list", "home.read", "home.write"]
}

skill "jira_read" {
  policy: "Fetch Jira issues, search with JQL, and download read-only attachments and images."
  allowed_tools: [
    "atlassian.jira_get_issue",
    "atlassian.jira_search",
    "atlassian.jira_get_project_issues",
    "atlassian.jira_get_issue_images",
    "atlassian.jira_download_attachments",
    "atlassian.jira_get_link_types",
    "atlassian.jira_get_all_projects",
    "atlassian.jira_search_fields"
  ]
}

skill "confluence_read" {
  policy: "Fetch Confluence pages, search with CQL, and download read-only attachments and images."
  allowed_tools: [
    "atlassian.confluence_search",
    "atlassian.confluence_get_page",
    "atlassian.confluence_get_page_children",
    "atlassian.confluence_get_page_history",
    "atlassian.confluence_get_page_images",
    "atlassian.confluence_get_attachments",
    "atlassian.confluence_download_attachment",
    "atlassian.confluence_download_content_attachments",
    "atlassian.confluence_get_comments"
  ]
}
