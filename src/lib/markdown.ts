import { Marked } from 'marked';
import DOMPurify from 'dompurify';

/** GitHub-flavored Markdown from the agent, rendered to sanitized HTML. */
const marked = new Marked({ gfm: true, breaks: true, async: false });

export function renderMarkdown(text: string): string {
  const html = marked.parse(text) as string;
  return DOMPurify.sanitize(html, {
    // Model output must never bring scripts, styles, forms or embedded content.
    FORBID_TAGS: ['style', 'form', 'input', 'button', 'iframe', 'object', 'embed', 'img', 'svg'],
    FORBID_ATTR: ['style'],
  });
}
