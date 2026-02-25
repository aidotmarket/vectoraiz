"""
Prompt Registry
===============

Jinja2-based template engine for RAG prompts.
Supports loading from filesystem with fallback to built-in defaults.

Phase: 3.V.4
Created: 2026-01-25
"""

import logging
from pathlib import Path
from typing import List, Optional

from jinja2 import Environment, FileSystemLoader, ChoiceLoader, DictLoader, select_autoescape, TemplateNotFound

logger = logging.getLogger(__name__)

# Default templates - ensures system works even if files are missing
DEFAULT_TEMPLATES = {
    "rag_qa": """**ROLE:**
You are a specialized AI assistant for vectorAIz. Your goal is to answer questions accurately based ONLY on the provided context.

**INSTRUCTIONS:**
1. Analyze the question and understand what the user is asking.
2. Review the context chunks below. Each is labeled with [N].
3. Answer using ONLY information from the context. Do not use external knowledge.
4. CITE YOUR SOURCES: At the end of every sentence that uses information from context, append the source number like [1] or [1][2].
5. If the context does not contain the answer, say: "I don't have information about that in the provided documents."

**CONTEXT:**
{% for source in context_chunks %}
[{{ source.index }}] {{ source.text }}
{% endfor %}

**USER QUESTION:**
{{ question }}

**ANSWER:**
""",

    "rag_summary": """Summarize the following content.

**CONTENT:**
{% for source in context_chunks %}
[{{ source.index }}] {{ source.text }}
{% endfor %}

**INSTRUCTIONS:**
- Provide a concise summary of the key points.
- Cite sources using [N] format.
- Focus on the most important information.

**SUMMARY:**
""",

    "rag_extract": """Extract specific information from the following content.

**CONTENT:**
{% for source in context_chunks %}
[{{ source.index }}] {{ source.text }}
{% endfor %}

**EXTRACTION REQUEST:**
{{ question }}

**INSTRUCTIONS:**
- Extract only the requested information.
- Cite sources using [N] format.
- If the information is not present, say "Not found in the provided documents."

**EXTRACTED INFORMATION:**
""",

    "rag_compare": """Compare and contrast information from the following sources.

**SOURCES:**
{% for source in context_chunks %}
[{{ source.index }}] {{ source.text }}
{% endfor %}

**COMPARISON REQUEST:**
{{ question }}

**INSTRUCTIONS:**
- Identify similarities and differences across the sources.
- Cite specific sources using [N] format.
- Be objective and balanced.

**COMPARISON:**
""",

    "setup_guide": """**ROLE:**
You are allAI, the friendly setup assistant for vectorAIz. You help users connect their favorite LLM clients (like ChatGPT, Claude, or custom apps) to their vectorAIz data.

**INSTRUCTIONS:**
1. Be warm, encouraging, and clear — you're guiding someone through a technical setup.
2. Use the setup knowledge below to answer the user's question.
3. If the user asks about connecting ChatGPT, walk them through Custom GPT Actions.
4. If the user asks about connecting Claude, walk them through MCP setup.
5. If they ask generally, give an overview of all options.
6. Always mention where to find their API key (Settings → API Keys in the vectorAIz dashboard).

**SETUP KNOWLEDGE:**
{{ setup_context }}

{% if context_chunks %}
**ADDITIONAL DATA CONTEXT:**
{% for source in context_chunks %}
[{{ source.index }}] {{ source.text }}
{% endfor %}
{% endif %}

**USER QUESTION:**
{{ question }}

**ANSWER:**
"""
}


class PromptRegistry:
    """
    Manages Jinja2 templates for LLM prompts.
    Supports loading from filesystem with fallback to internal defaults.
    """

    def __init__(self, template_dir: Optional[str] = None):
        """
        Initialize the prompt registry.
        
        Args:
            template_dir: Optional path to template directory
        """
        self.template_dir = Path(template_dir) if template_dir else Path("app/templates/prompts")
        self._setup_environment()

    def _setup_environment(self):
        """Initialize Jinja2 environment with FileSystemLoader and fallback DictLoader."""
        loaders = []
        
        # 1. Try filesystem loader if directory exists
        if self.template_dir.exists():
            loaders.append(FileSystemLoader(str(self.template_dir)))
            logger.info(f"Loaded templates from {self.template_dir}")
        else:
            logger.warning(f"Template directory {self.template_dir} not found. Using defaults only.")

        # 2. Always include default loader as fallback
        loaders.append(DictLoader(DEFAULT_TEMPLATES))

        self.env = Environment(
            loader=ChoiceLoader(loaders),
            autoescape=select_autoescape(),
            trim_blocks=True,
            lstrip_blocks=True
        )

    def render(self, template_name: str, **variables) -> str:
        """
        Render a template by name with provided variables.
        
        Args:
            template_name: Name of the template (e.g., 'rag_qa')
            **variables: Variables to inject into the template
            
        Returns:
            Rendered string
            
        Raises:
            ValueError: If template is not found
        """
        try:
            # Try exact match first
            try:
                template = self.env.get_template(template_name)
            except TemplateNotFound:
                # Try with .j2 extension for filesystem templates
                if not template_name.endswith('.j2'):
                    try:
                        template = self.env.get_template(f"{template_name}.j2")
                    except TemplateNotFound:
                        raise
                else:
                    raise

            return template.render(**variables)
            
        except TemplateNotFound:
            logger.error(f"Template '{template_name}' not found.")
            raise ValueError(f"Template '{template_name}' not found. Available: {self.list_templates()}")
        except Exception as e:
            logger.error(f"Error rendering template '{template_name}': {e}")
            raise

    def register_template(self, name: str, template_content: str):
        """
        Register a custom template at runtime.
        
        Args:
            name: Template name
            template_content: Jinja2 template string
        """
        DEFAULT_TEMPLATES[name] = template_content
        # Rebuild environment to include new template
        self._setup_environment()
        logger.info(f"Registered custom template: {name}")

    def list_templates(self) -> List[str]:
        """List available template names."""
        templates = set(DEFAULT_TEMPLATES.keys())
        try:
            templates.update(self.env.list_templates())
        except Exception:
            pass
        return sorted(templates)
    
    def get_template_info(self, template_name: str) -> dict:
        """Get metadata about a template."""
        return {
            "name": template_name,
            "is_default": template_name in DEFAULT_TEMPLATES,
            "available": template_name in self.list_templates(),
        }


# Singleton instance
_prompt_registry: Optional[PromptRegistry] = None


def get_prompt_registry() -> PromptRegistry:
    """Get the singleton prompt registry instance."""
    global _prompt_registry
    if _prompt_registry is None:
        _prompt_registry = PromptRegistry()
    return _prompt_registry


def reset_prompt_registry():
    """Reset the singleton (useful for testing)."""
    global _prompt_registry
    _prompt_registry = None
