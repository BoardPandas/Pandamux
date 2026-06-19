import { useCallback, useMemo } from 'react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { openInWmuxBrowser } from '../../utils/open-in-browser';
import '../../styles/markdown.css';

interface MarkdownPaneProps {
  content?: string;
  surfaceId: string;
}

export default function MarkdownPane({ content = '', surfaceId }: MarkdownPaneProps) {
  const html = useMemo(() => {
    if (!content) return '<p style="opacity: 0.5">No content. Use wmux markdown set to add content.</p>';

    marked.setOptions({
      gfm: true,
      breaks: true,
    });

    // Markdown can arrive from untrusted sources (CLI/pipe callers, agents,
    // loaded files). marked emits raw HTML, so sanitize before injecting it
    // via dangerouslySetInnerHTML to prevent XSS in the renderer (which has
    // preload/IPC access). FORBID javascript: URIs and event handlers.
    const rawHtml = marked.parse(content) as string;
    return DOMPurify.sanitize(rawHtml, {
      USE_PROFILES: { html: true },
      FORBID_TAGS: ['style', 'form', 'input', 'button', 'textarea', 'select'],
      FORBID_ATTR: ['style'],
    });
  }, [content]);

  const handleClick = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    const anchor = (event.target as HTMLElement)?.closest?.('a') as HTMLAnchorElement | null;
    if (!anchor?.href) return;

    event.preventDefault();
    const forceExternal = event.ctrlKey || event.metaKey;
    openInWmuxBrowser(anchor.href, { forceExternal });
  }, []);

  return (
    <div className="markdown-pane" data-surface-id={surfaceId}>
      <div
        className="markdown-pane__content"
        onClick={handleClick}
        dangerouslySetInnerHTML={{ __html: html }}
      />
    </div>
  );
}
