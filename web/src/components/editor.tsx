// Tiptap rich-text editor that reads and writes Markdown. Citations are inline atom nodes
// stored in Markdown as [[E12]]; they render as chips and are removed on export and publish.

import { Node, mergeAttributes } from '@tiptap/core'
import { TableKit } from '@tiptap/extension-table'
import { Markdown } from '@tiptap/markdown'
import { EditorContent, useEditor, type Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { Bold, Heading2, Heading3, Italic, List, ListOrdered, Quote, Redo2, Table as TableIcon, Undo2 } from 'lucide-react'
import { useEffect } from 'react'
import { cn } from '@/lib/utils'

export const Citation = Node.create({
  name: 'citation',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return { num: { default: 0, parseHTML: (el) => Number(el.getAttribute('data-citation')) } }
  },
  parseHTML() {
    return [{ tag: 'span[data-citation]' }]
  },
  renderHTML({ node, HTMLAttributes }) {
    return ['span', mergeAttributes(HTMLAttributes, { 'data-citation': node.attrs.num, class: 'citation-chip', title: `Library entry E${node.attrs.num}` }), `E${node.attrs.num}`]
  },
  renderText({ node }) {
    return `[[E${node.attrs.num}]]`
  },
  markdownTokenizer: {
    name: 'citation',
    level: 'inline',
    start: (src: string) => src.indexOf('[[E'),
    tokenize(src: string) {
      const m = /^\[\[E(\d+)\]\]/.exec(src)
      if (!m) return undefined
      return { type: 'citation', raw: m[0], num: Number(m[1]) }
    },
  },
  parseMarkdown(token) {
    return { type: 'citation', attrs: { num: Number((token as { num?: number }).num ?? 0) } }
  },
  renderMarkdown(node) {
    return `[[E${node.attrs?.num ?? 0}]]`
  },
})

function ToolButton({ onClick, active, label, children }: { onClick: () => void; active?: boolean; label: string; children: React.ReactNode }) {
  return (
    <button type="button" onClick={onClick} aria-label={label} title={label} className={cn('rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground [&_svg]:size-4', active && 'bg-accent text-primary')}>
      {children}
    </button>
  )
}

function Toolbar({ editor }: { editor: Editor }) {
  return (
    <div className="flex flex-wrap items-center gap-0.5 border-b px-2 py-1.5">
      <ToolButton label="Heading" active={editor.isActive('heading', { level: 2 })} onClick={() => editor.chain().focus().toggleHeading({ level: 2 }).run()}><Heading2 /></ToolButton>
      <ToolButton label="Subheading" active={editor.isActive('heading', { level: 3 })} onClick={() => editor.chain().focus().toggleHeading({ level: 3 }).run()}><Heading3 /></ToolButton>
      <ToolButton label="Bold" active={editor.isActive('bold')} onClick={() => editor.chain().focus().toggleBold().run()}><Bold /></ToolButton>
      <ToolButton label="Italic" active={editor.isActive('italic')} onClick={() => editor.chain().focus().toggleItalic().run()}><Italic /></ToolButton>
      <ToolButton label="Bullet list" active={editor.isActive('bulletList')} onClick={() => editor.chain().focus().toggleBulletList().run()}><List /></ToolButton>
      <ToolButton label="Numbered list" active={editor.isActive('orderedList')} onClick={() => editor.chain().focus().toggleOrderedList().run()}><ListOrdered /></ToolButton>
      <ToolButton label="Quote" active={editor.isActive('blockquote')} onClick={() => editor.chain().focus().toggleBlockquote().run()}><Quote /></ToolButton>
      <ToolButton label="Insert table" onClick={() => editor.chain().focus().insertTable({ rows: 3, cols: 2, withHeaderRow: true }).run()}><TableIcon /></ToolButton>
      <span className="mx-1 h-5 w-px bg-border" />
      <ToolButton label="Undo" onClick={() => editor.chain().focus().undo().run()}><Undo2 /></ToolButton>
      <ToolButton label="Redo" onClick={() => editor.chain().focus().redo().run()}><Redo2 /></ToolButton>
    </div>
  )
}

export function MarkdownEditor({
  value,
  onChange,
  onCitationClick,
  onReady,
  className,
}: {
  value: string
  onChange: (markdown: string) => void
  onCitationClick?: (num: number) => void
  onReady?: (editor: Editor) => void
  className?: string
}) {
  const editor = useEditor({
    extensions: [StarterKit.configure({ heading: { levels: [2, 3] }, link: { openOnClick: false } }), TableKit, Citation, Markdown],
    content: value,
    contentType: 'markdown',
    editorProps: {
      attributes: { class: 'prose-editor px-5 py-4', 'aria-label': 'Post body', role: 'textbox', 'aria-multiline': 'true' },
      handleClickOn: (_view, _pos, node) => {
        if (node.type.name === 'citation') {
          onCitationClick?.(node.attrs.num)
          return true
        }
        return false
      },
    },
    onUpdate: ({ editor }) => onChange(editor.getMarkdown()),
  })
  useEffect(() => {
    if (editor) onReady?.(editor)
  }, [editor, onReady])
  if (!editor) return null
  return (
    <div className={cn('rounded-xl border bg-card', className)}>
      <Toolbar editor={editor} />
      <EditorContent editor={editor} />
    </div>
  )
}
