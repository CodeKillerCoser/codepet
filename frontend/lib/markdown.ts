import DOMPurify from "dompurify";
import { Marked } from "marked";

const markdown = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    // Task checkboxes are display content, never editable controls.
    checkbox({ checked }) { return checked ? "☑ " : "☐ "; },
    // Keep alt text without loading remote resources in a desktop overlay.
    image({ text }) { return text.replace(/[&<>"']/g, escapeCharacter); },
  },
});

function escapeCharacter(character: string): string {
  return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[character]!;
}

export function renderMessageMarkdown(message: string): string {
  const html = markdown.parse(message, { async: false });
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: ["p", "br", "strong", "em", "del", "code", "pre", "blockquote", "ul", "ol", "li", "a", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "table", "thead", "tbody", "tr", "th", "td"],
    ALLOWED_ATTR: ["href", "title", "start"],
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
    ALLOWED_URI_REGEXP: /^(?:https?:\/\/|mailto:)/i,
  });
}
