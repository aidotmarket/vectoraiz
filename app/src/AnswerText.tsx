import type { ReactNode } from "react";

// Only these Markdown constructs become elements. Everything else, including
// HTML and links, stays React text and cannot navigate the webview.
function inline(text: string): ReactNode[] {
  return text.split(/(\*\*[^*]+\*\*|__[^_]+__|\*[^*]+\*|_[^_]+_)/g).map((part, index) => {
    if ((part.startsWith("**") && part.endsWith("**")) || (part.startsWith("__") && part.endsWith("__"))) {
      return <strong key={index}>{inline(part.slice(2, -2))}</strong>;
    }
    if (part.length > 2 && ((part.startsWith("*") && part.endsWith("*")) || (part.startsWith("_") && part.endsWith("_")))) {
      return <em key={index}>{part.slice(1, -1)}</em>;
    }
    return part;
  });
}

export function AnswerText({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  const lines = text.split("\n");
  for (let i = 0; i < lines.length;) {
    const list = /^\s*(?:[-+*]|\d+\.)\s+/.exec(lines[i]);
    if (list) {
      const start = i;
      const ordered = /\d+\./.test(list[0]);
      const items: ReactNode[] = [];
      while (i < lines.length) {
        const match = /^\s*(?:[-+*]|\d+\.)\s+/.exec(lines[i]);
        if (!match || /\d+\./.test(match[0]) !== ordered) break;
        items.push(<li key={i}>{inline(lines[i].slice(match[0].length))}</li>);
        i++;
      }
      blocks.push(ordered ? <ol className="list-decimal pl-6" key={start}>{items}</ol> : <ul className="list-disc pl-6" key={start}>{items}</ul>);
    } else {
      const start = i;
      const paragraph: string[] = [];
      do { paragraph.push(lines[i++]); }
      while (i < lines.length && !/^\s*(?:[-+*]|\d+\.)\s+/.test(lines[i]));
      blocks.push(<p className="whitespace-pre-wrap" key={start}>{inline(paragraph.join("\n"))}</p>);
    }
  }
  return blocks;
}
