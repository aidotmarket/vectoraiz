import type { ReactNode } from "react";

// Only these Markdown constructs become elements. Everything else, including
// HTML and links, stays React text and cannot navigate the webview.
function inline(text: string, streaming = false): ReactNode[] {
  if (streaming) {
    const complete: {index: number; text: string}[] = [];
    let opener: {index: number; text: string} | undefined;
    let consumed = 0;
    for (const marker of text.matchAll(/\*\*|__|\*|_/g)) {
      const index = marker.index!;
      if (index < consumed) continue;
      if (/\w/.test(text[index - 1] ?? "") && /\w/.test(text[index + marker[0].length] ?? "")) continue;
      // A single marker may be the first half of a double closer.
      if (opener?.text.length === 2 && marker[0] === opener.text[0]) continue;
      const closed = /^(\*\*[^*]+\*\*|__[^_]+__|\*[^*]+\*|_[^_]+_)/.exec(text.slice(index));
      if (closed) {
        complete.push({index, text: closed[0]});
        consumed = index + closed[0].length;
      } else {
        opener = {index, text: marker[0]};
      }
    }
    const nodes: ReactNode[] = [];
    let offset = 0;
    for (const span of complete) {
      if (opener && span.index > opener.index) break;
      nodes.push(text.slice(offset, span.index), inline(span.text));
      offset = span.index + span.text.length;
    }
    if (opener) nodes.push(text.slice(offset, opener.index), text.slice(opener.index + opener.text.length));
    else nodes.push(text.slice(offset));
    return nodes;
  }
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

export function AnswerText({ text, streaming = false }: { text: string; streaming?: boolean }) {
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
        items.push(<li key={i}>{inline(lines[i].slice(match[0].length), streaming)}</li>);
        i++;
      }
      blocks.push(ordered ? <ol className="list-decimal pl-6" key={start}>{items}</ol> : <ul className="list-disc pl-6" key={start}>{items}</ul>);
    } else {
      const start = i;
      const paragraph: string[] = [];
      do { paragraph.push(lines[i++]); }
      while (i < lines.length && !/^\s*(?:[-+*]|\d+\.)\s+/.test(lines[i]));
      blocks.push(<p className="whitespace-pre-wrap" key={start}>{inline(paragraph.join("\n"), streaming)}</p>);
    }
  }
  return blocks;
}
