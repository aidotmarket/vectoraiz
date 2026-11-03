import type { ReactNode } from "react";

// Only these Markdown constructs become elements. Everything else, including
// HTML and links, stays React text and cannot navigate the webview.
function inline(text: string, streaming = false): ReactNode[] {
  if (streaming) {
    // Consume complete emphasis first, then find the last opener still waiting
    // for its closing marker. Its unfinished contents stay plain text.
    const markers = text.matchAll(/\*\*[^*]+\*\*|__[^_]+__|\*[^*]+\*|_[^_]+_|\*\*|__|\*|_/g);
    let opener: RegExpMatchArray | undefined;
    for (const marker of markers) {
      if (/^(\*\*|__|\*|_)$/.test(marker[0])) opener = marker;
    }
    if (opener) return [...inline(text.slice(0, opener.index)), text.slice(opener.index! + opener[0].length)];
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
