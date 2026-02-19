"""
allAI Tool Definitions — Anthropic tool-use format.

Defines the tools available to allAI during the agentic loop.
These are passed to the LLM in the Anthropic Messages API `tools` parameter.

[COUNCIL] delete_dataset has NO confirm param — routed through ConfirmationService.

PHASE: BQ-ALLAI-B1 — Tool Definitions
CREATED: 2026-02-16
"""

ALLAI_TOOLS = [
    {
        "name": "list_datasets",
        "description": (
            "List all datasets in the user's vectorAIz instance with metadata "
            "(name, file type, row count, column count, status). "
            "Use when the user asks about their data, files, or uploads."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "status_filter": {
                    "type": "string",
                    "enum": ["all", "ready", "processing", "error"],
                    "description": "Filter by processing status. Default: all",
                },
            },
            "required": [],
        },
    },
    {
        "name": "get_dataset_detail",
        "description": (
            "Get detailed info about a specific dataset: columns, types, row count, "
            "file size. Use when the user asks about a specific file's structure."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {
                    "type": "string",
                    "description": "The dataset ID",
                },
            },
            "required": ["dataset_id"],
        },
    },
    {
        "name": "preview_rows",
        "description": (
            "Show sample rows from a dataset as an inline table in the chat. "
            "Use when user says 'show me the data', 'preview', 'what does it look like'. "
            "Data is rendered as a table — do NOT repeat the row values as text."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {
                    "type": "string",
                    "description": "The dataset ID to preview",
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Number of rows to show (default 10)",
                },
            },
            "required": ["dataset_id"],
        },
    },
    {
        "name": "run_sql_query",
        "description": (
            "Execute a READ-ONLY SQL query via DuckDB. "
            "Tables are named dataset_{dataset_id}. "
            "Results are shown as an inline table — do NOT repeat raw data as text. "
            "Only SELECT queries are allowed."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "SQL SELECT query. Tables: dataset_{dataset_id}",
                },
                "limit": {
                    "type": "integer",
                    "maximum": 200,
                    "description": "Maximum rows to return (default 50)",
                },
            },
            "required": ["query"],
        },
    },
    {
        "name": "search_vectors",
        "description": (
            "Semantic search across vectorized datasets. "
            "Returns relevant chunks matching a natural language query. "
            "Use for fuzzy/meaning-based search rather than exact SQL queries."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query",
                },
                "dataset_id": {
                    "type": "string",
                    "description": "Limit search to a specific dataset (optional)",
                },
                "limit": {
                    "type": "integer",
                    "maximum": 20,
                    "description": "Maximum results to return (default 5)",
                },
            },
            "required": ["query"],
        },
    },
    {
        "name": "get_system_status",
        "description": (
            "Get system health info: Qdrant status, DuckDB availability, "
            "LLM provider, connected mode. Use when user asks about system health."
        ),
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": [],
        },
    },
    {
        "name": "get_dataset_statistics",
        "description": (
            "Get statistical profile of a dataset: min, max, mean, median, "
            "null counts, unique values per column. Use when user asks about "
            "data distribution, statistics, or data quality."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {
                    "type": "string",
                    "description": "The dataset ID to profile",
                },
            },
            "required": ["dataset_id"],
        },
    },
    {
        "name": "delete_dataset",
        "description": (
            "Request deletion of a dataset and all its processed data. "
            "This triggers a confirmation prompt to the user — allAI cannot "
            "execute deletion directly. Use only when the user explicitly asks to delete."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "dataset_id": {
                    "type": "string",
                    "description": "The dataset ID to delete",
                },
            },
            "required": ["dataset_id"],
        },
    },
    # ------------------------------------------------------------------
    # BQ-MCP-RAG Phase 2: External Connectivity Tools
    # ------------------------------------------------------------------
    {
        "name": "connectivity_status",
        "description": (
            "Check external connectivity state: whether it's enabled, list of tokens "
            "(id, label, scopes, last_used, secret_last4 — NEVER full secret), "
            "usage metrics, and recent query activity. Use when the user asks about "
            "their external AI connections or connectivity setup."
        ),
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": [],
        },
    },
    {
        "name": "connectivity_enable",
        "description": (
            "Enable external connectivity so other AI tools (Claude Desktop, ChatGPT, "
            "Cursor, etc.) can query the user's vectorAIz data via MCP or REST API. "
            "Use when the user wants to connect external AI tools."
        ),
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": [],
        },
    },
    {
        "name": "connectivity_disable",
        "description": (
            "Disable external connectivity. Rejects all external requests but preserves "
            "existing tokens for re-enabling later. Use when the user wants to stop "
            "external access."
        ),
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": [],
        },
    },
    {
        "name": "connectivity_create_token",
        "description": (
            "Create a new connectivity token with a label and scopes for an external AI tool. "
            "The full token is shown ONCE — the user MUST save it immediately. "
            "Use when connecting a new AI platform to vectorAIz."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Human-readable label for the token (e.g. 'Claude Desktop', 'Cursor')",
                },
                "scopes": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": (
                        "Permission scopes. Options: ext:search, ext:sql, ext:schema, ext:datasets. "
                        "Default: all scopes."
                    ),
                },
            },
            "required": ["label"],
        },
    },
    {
        "name": "connectivity_revoke_token",
        "description": (
            "Revoke a connectivity token by its token ID. The token will immediately "
            "stop working. Use when a token is compromised or no longer needed."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "token_id": {
                    "type": "string",
                    "description": "The 8-character token ID to revoke",
                },
            },
            "required": ["token_id"],
        },
    },
    {
        "name": "connectivity_generate_setup",
        "description": (
            "Generate platform-specific setup instructions for connecting an external AI tool "
            "to vectorAIz. Includes exact config, copy-paste commands, and troubleshooting tips. "
            "Supported platforms: claude_desktop, chatgpt_desktop, cursor, gemini, vscode, "
            "openai_custom_gpt, generic_rest, generic_llm."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "platform": {
                    "type": "string",
                    "enum": [
                        "claude_desktop",
                        "chatgpt_desktop",
                        "cursor",
                        "gemini",
                        "vscode",
                        "openai_custom_gpt",
                        "generic_rest",
                        "generic_llm",
                    ],
                    "description": "Target platform for setup instructions",
                },
                "token": {
                    "type": "string",
                    "description": "The full connectivity token (vzmcp_...) to embed in config",
                },
                "base_url": {
                    "type": "string",
                    "description": "Base URL for the vectorAIz instance (default: http://localhost:8100)",
                },
            },
            "required": ["platform", "token"],
        },
    },
    {
        "name": "connectivity_test",
        "description": (
            "Run a self-diagnostic on external connectivity: validate a token, check MCP "
            "responding, count accessible datasets, run a sample query, and measure latency. "
            "Use after setup to verify everything works."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "The full connectivity token (vzmcp_...) to test",
                },
            },
            "required": ["token"],
        },
    },
]
