import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

// Render assistant chat text as GitHub-flavored markdown (headings, bold, lists, tables, code) with Tailwind
// styling. Links render as plain styled text (not navigable) so a click can't navigate the app's own webview
// away from itself. Used by both the full ChatView and the compact QuickChat.
const components: Components = {
  p: ({ children }) => <p className="mb-2 leading-relaxed last:mb-0">{children}</p>,
  h1: ({ children }) => (
    <h1 className="mb-2 mt-3 text-lg font-semibold first:mt-0">{children}</h1>
  ),
  h2: ({ children }) => (
    <h2 className="mb-2 mt-3 text-base font-semibold first:mt-0">{children}</h2>
  ),
  h3: ({ children }) => (
    <h3 className="mb-1 mt-2 text-sm font-semibold first:mt-0">{children}</h3>
  ),
  ul: ({ children }) => (
    <ul className="mb-2 ml-5 list-disc space-y-1 last:mb-0">{children}</ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-2 ml-5 list-decimal space-y-1 last:mb-0">{children}</ol>
  ),
  li: ({ children }) => <li className="leading-relaxed">{children}</li>,
  strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
  em: ({ children }) => <em className="italic">{children}</em>,
  a: ({ children }) => (
    <span className="text-emerald-600 underline dark:text-emerald-400">
      {children}
    </span>
  ),
  blockquote: ({ children }) => (
    <blockquote className="mb-2 border-l-2 border-neutral-300 pl-3 text-neutral-600 last:mb-0 dark:border-neutral-600 dark:text-neutral-300">
      {children}
    </blockquote>
  ),
  code: ({ className, children }) => {
    const isBlock = /language-/.test(className || "");
    return isBlock ? (
      <code className="block overflow-x-auto rounded-md bg-neutral-100 p-2 font-mono text-[13px] dark:bg-neutral-800">
        {children}
      </code>
    ) : (
      <code className="rounded bg-neutral-100 px-1 py-0.5 font-mono text-[13px] dark:bg-neutral-800">
        {children}
      </code>
    );
  },
  pre: ({ children }) => (
    <pre className="mb-2 overflow-x-auto last:mb-0">{children}</pre>
  ),
  table: ({ children }) => (
    <div className="mb-2 overflow-x-auto last:mb-0">
      <table className="w-full border-collapse text-[13px]">{children}</table>
    </div>
  ),
  th: ({ children }) => (
    <th className="border border-neutral-300 px-2 py-1 text-left font-semibold dark:border-neutral-600">
      {children}
    </th>
  ),
  td: ({ children }) => (
    <td className="border border-neutral-300 px-2 py-1 dark:border-neutral-600">
      {children}
    </td>
  ),
  hr: () => <hr className="my-3 border-neutral-200 dark:border-neutral-700" />,
};

export function ChatMarkdown({ children }: { children: string }) {
  return (
    <div className="text-neutral-800 dark:text-neutral-100">
      <Markdown remarkPlugins={[remarkGfm]} components={components}>
        {children}
      </Markdown>
    </div>
  );
}
