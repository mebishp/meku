# Live Markdown Editor: Feature Checklist

Scope: the editor only (text input, live rendering, keyboard behavior, and editing tools). Excludes file explorer, graph view, plugins marketplace, sync, and publishing.

---

## 1. Core Text Input

- [ ] Plain text typing with IME support (Chinese, Japanese, Korean, etc.)
- [ ] Undo / redo with sensible history grouping (typing bursts become one undo step)
- [ ] Cut, copy, paste (plain text and rich-to-markdown conversion)
- [ ] Select all, select word, select line, select paragraph
- [ ] Expand selection (Ctrl+Shift+Right, or repeated Ctrl+A style expansion)
- [ ] Multiple cursors (Ctrl+Alt+Up/Down, Ctrl+D to select next occurrence)
- [ ] Column / box selection with Alt+drag
- [ ] Soft wrap toggle
- [ ] Line numbers toggle (optional, source mode)
- [ ] Spellcheck with underline and right-click suggestions
- [ ] Auto-save or debounced save with dirty-state indicator

---

## 2. Word and Line Navigation

- [ ] Ctrl+Left / Ctrl+Right jump by word (respects camelCase, punctuation, and markdown syntax)
- [ ] Ctrl+Backspace deletes previous word
- [ ] Ctrl+Delete deletes next word
- [ ] Ctrl+Shift+Backspace deletes to line start (or previous word when at line start)
- [ ] Home goes to first non-whitespace character, press again for column 0
- [ ] End goes to end of line
- [ ] Ctrl+Home / Ctrl+End go to document start and end
- [ ] Page Up / Page Down
- [ ] Go to line (Ctrl+G)
- [ ] Jump to heading (from outline)
- [ ] Bookmarks or jump to previous cursor position (Alt+Left/Right style)

---

## 3. Line Editing Shortcuts

- [ ] Move line up / down (Alt+Up / Alt+Down)
- [ ] Duplicate line or selection (Ctrl+Shift+D)
- [ ] Delete line (Ctrl+Shift+K)
- [ ] Insert line below (Ctrl+Enter) and above (Ctrl+Shift+Enter)
- [ ] Join lines (Ctrl+J)
- [ ] Toggle comment in code (Ctrl+/)
- [ ] Sort selected lines
- [ ] Transform case: upper, lower, title, toggle

---

## 4. Enter and Smart Continuation

- [ ] Enter in a bullet list creates the next bullet
- [ ] Enter on an empty bullet exits the list
- [ ] Enter in a numbered list increments the number
- [ ] Renumber the list automatically when an item is inserted or removed
- [ ] Enter in a task list (`- [ ]`) creates an unchecked task
- [ ] Enter in a blockquote continues the `>` prefix
- [ ] Enter in a callout continues the callout prefix
- [ ] Enter inside a code block inserts a newline without leaving the block
- [ ] Enter in a table moves to the next row, or adds a row at the end
- [ ] Enter preserves indentation of the current line
- [ ] Shift+Enter inserts a soft line break (inside lists and quotes)
- [ ] Enter after a heading starts a normal paragraph

---

## 5. Indentation and Lists

- [ ] Tab indents the current line or list item
- [ ] Shift+Tab outdents
- [ ] Indenting a list item moves its nested children with it
- [ ] Tab on a multi-line selection indents every selected line
- [ ] Backspace at the start of a list item removes one indent level
- [ ] Convert line to bullet, numbered, or task (commands)
- [ ] Toggle task checkbox (Ctrl+Enter or click)
- [ ] Fold and unfold list items with children
- [ ] Ordered list renumbering after reorder

---

## 6. Auto-Pairing and Smart Typing

- [ ] Auto-close brackets: `()`, `[]`, `{}`
- [ ] Auto-close quotes: `""`, `''`, and backticks
- [ ] Auto-close markdown markers: `**`, `*`, `==`, `~~`, `` ` ``
- [ ] Typing a closing character over an existing one skips it
- [ ] Wrap selection with a marker when typing `*`, `_`, `` ` ``, `[`, `(` with text selected
- [ ] Backspace inside an empty pair deletes both characters
- [ ] Auto-close `[[` for wikilinks with autocomplete
- [ ] Smart quotes option (off by default)
- [ ] Smart dashes option (`--` to em dash, optional)
- [ ] Autocomplete brackets can be toggled off in settings

---

## 7. Live Preview Rendering (Core Obsidian Feel)

- [ ] Render markdown inline while typing, not in a separate pane
- [ ] Hide markdown syntax (`**`, `#`, `_`, `` ` ``, `[]()`) when the cursor is not on that line or span
- [ ] Reveal raw syntax when the cursor enters the element
- [ ] Headings styled by level (H1 to H6), with `#` shown only on the active line
- [ ] Bold, italic, strikethrough, highlight (`==text==`), inline code
- [ ] Superscript and subscript (where supported)
- [ ] Links rendered as clickable text, with the URL hidden until focused
- [ ] Images rendered inline with `![alt](src)` and `![[embed]]`
- [ ] Horizontal rules rendered as lines
- [ ] Blockquotes styled with a left border
- [ ] Callouts (`> [!note]`, `> [!warning]`, etc.) with collapsible `+` and `-` states
- [ ] Code blocks with syntax highlighting and a language label
- [ ] Inline LaTeX math (`$...$`) and block math (`$$...$$`) rendered live
- [ ] Mermaid diagrams rendered from fenced code blocks (optional)
- [ ] Tables rendered as grids with alignment respected
- [ ] Footnotes `[^1]` rendered with hover or click preview
- [ ] Embedded notes and transclusion (`![[note#heading]]`)
- [ ] Comments `%% hidden %%` hidden from preview
- [ ] Emoji shortcodes rendered (`:smile:`)
- [ ] Task checkboxes rendered as clickable boxes

---

## 8. Source, Live, and Reading Modes

- [ ] Source mode: raw markdown with syntax highlighting, no rendering
- [ ] Live preview mode: rendered-as-you-type (default)
- [ ] Reading mode: fully rendered, read-only view
- [ ] Toggle between modes with a shortcut (Ctrl+E)
- [ ] Mode preserved per document
- [ ] Reading mode respects the same theme and typography

---

## 9. Links, Wikilinks, and Embeds

- [ ] `[[wikilink]]` syntax with autocomplete from known files and headings
- [ ] `[[link|alias]]` display text
- [ ] `[[note#heading]]` and `[[note#^block-id]]` anchors
- [ ] Block IDs (`^id`) created and inserted by command
- [ ] Ctrl+Click (or Cmd+Click) follows a link
- [ ] Hover preview of linked notes
- [ ] Paste a URL over selected text creates `[text](url)`
- [ ] Paste a URL alone keeps it as a bare link (or converts to a title, optional)
- [ ] Broken link highlighting when the target does not exist
- [ ] Autocomplete for external URLs from history (optional)
- [ ] Backlinks display (side panel, optional for editor scope)

---

## 10. Tags, Properties, and Metadata

- [ ] `#tag` and nested `#tag/subtag` syntax with autocomplete
- [ ] Tag highlighting in live preview
- [ ] YAML frontmatter (`---` block) parsed as properties
- [ ] Properties UI for frontmatter: text, number, date, checkbox, list, tags
- [ ] Editing frontmatter as raw YAML in source mode
- [ ] Aliases and titles from frontmatter

---

## 11. Slash Commands and Command Palette

- [ ] Type `/` to open an inline command menu
- [ ] Commands for headings, lists, task, quote, callout, code block, table, divider, math block, image, link, date, time
- [ ] Fuzzy search inside the menu
- [ ] Keyboard navigation (arrow keys, Enter, Escape)
- [ ] Command palette (Ctrl+P or Cmd+P) listing every editor command
- [ ] Every command is bindable to a custom hotkey
- [ ] Recently used commands ranked higher

---

## 12. Formatting Shortcuts

- [ ] Ctrl+B toggles bold
- [ ] Ctrl+I toggles italic
- [ ] Ctrl+Shift+S or Ctrl+Shift+X toggles strikethrough
- [ ] Ctrl+K inserts or edits a link
- [ ] Ctrl+` toggles inline code
- [ ] Ctrl+1 to Ctrl+6 sets heading level (toggles off when repeated)
- [ ] Ctrl+Shift+Enter or Ctrl+Shift+Alt+Enter for quote or callout toggle
- [ ] Formatting toggles work on the current word when no selection exists
- [ ] Clear formatting command

---

## 13. Tables

- [ ] Insert table with a size picker or command
- [ ] Tab moves to the next cell, Shift+Tab to the previous
- [ ] Tab on the last cell adds a new row
- [ ] Add and remove rows and columns from a context menu
- [ ] Column alignment toggle (left, center, right)
- [ ] Sort rows by a column
- [ ] Auto-format table pipes and padding when editing
- [ ] Paste CSV or TSV converts to a markdown table
- [ ] Cell text supports inline formatting

---

## 14. Code Blocks

- [ ] Fenced blocks with language identifier
- [ ] Syntax highlighting for at least 50 common languages
- [ ] Language selector and autocomplete for the fence language
- [ ] Copy button on code blocks
- [ ] Tab inserts spaces (or tab character, per setting) inside code blocks
- [ ] Auto-indent on Enter within code
- [ ] Bracket matching inside code
- [ ] Inline code with backticks, and double backticks for code containing backticks
- [ ] Line highlighting and line numbers (optional)
- [ ] Code folding within blocks (optional)

---

## 15. Headings, Outline, and Structure

- [ ] Outline panel generated from headings
- [ ] Click a heading in the outline to jump to it
- [ ] Fold and unfold any heading section
- [ ] Fold and unfold all headings (command)
- [ ] Heading anchors generated for links
- [ ] Sticky breadcrumb showing the current heading path (optional)
- [ ] Promote and demote heading level

---

## 16. Find, Replace, and Search

- [ ] Find in document (Ctrl+F) with highlight of all matches
- [ ] Replace and replace all (Ctrl+H)
- [ ] Case-sensitive, whole-word, and regex toggles
- [ ] Match count display and next/previous navigation
- [ ] Find respects current selection (search in selection)
- [ ] Preserve captured groups in regex replace
- [ ] Search results highlight in both source and preview modes

---

## 17. Images and Media

- [ ] Drag and drop an image into the editor to insert and save it
- [ ] Paste an image from clipboard to insert and save it
- [ ] Image insertion writes to a configured attachment folder
- [ ] Resize images with a size suffix (`![[img.png|300]]`)
- [ ] Drag image handle to resize (optional)
- [ ] Video and audio embeds (optional)
- [ ] PDF embed (optional)
- [ ] Image captions via alt text

---

## 18. Paste and Clipboard Handling

- [ ] Paste plain text as-is
- [ ] Paste HTML converted to markdown (headings, lists, links, bold, italic, code)
- [ ] Paste with Ctrl+Shift+V to force plain text
- [ ] Paste a URL over a selection creates a link
- [ ] Paste a list into a list continues the list correctly
- [ ] Paste code keeps indentation
- [ ] Paste a table from a spreadsheet creates a markdown table
- [ ] Copy as markdown vs copy as rendered text (both available)

---

## 19. Selection and Movement Refinements

- [ ] Shift+Arrow extends selection
- [ ] Ctrl+Shift+Left/Right extends by word
- [ ] Double-click selects word, triple-click selects line
- [ ] Shift+click extends selection
- [ ] Drag selected text to move it (with optional Ctrl to copy)
- [ ] Escape clears selection and multi-cursors

---

## 20. Visual and Theme

- [ ] Light and dark themes, following system setting
- [ ] Custom theme variables for fonts, colors, spacing
- [ ] Adjustable font family, size, and line height
- [ ] Editor width options: narrow, wide, full
- [ ] Centered text column option
- [ ] Zoom in and out (Ctrl+= / Ctrl+-) without changing file content
- [ ] Focus mode (dims all but the current line or paragraph)
- [ ] Typewriter mode (keeps the cursor vertically centered)
- [ ] Distraction-free full-screen mode
- [ ] Show or hide frontmatter in live preview
- [ ] Syntax highlighting colors for markdown tokens in source mode

---

## 21. Status and Feedback

- [ ] Word count, character count, and reading time
- [ ] Selection word and character count
- [ ] Line and column position indicator
- [ ] Save status indicator
- [ ] Non-blocking error toast for failed saves

---

## 22. Accessibility and Input Options

- [ ] Full keyboard operation of every feature
- [ ] Screen reader labels on controls and rendered elements
- [ ] Visible focus indicators
- [ ] Respect OS text size and reduced-motion settings
- [ ] Vim keybinding mode (optional)
- [ ] Emacs keybinding mode (optional)
- [ ] Customizable keymap with conflict detection
- [ ] Right-to-left text support
- [ ] Hyphenation and language setting for spellcheck

---

## 23. Performance and Robustness

- [ ] Handles documents of 100,000+ lines without input lag
- [ ] Incremental re-rendering of only changed blocks
- [ ] Rendering runs off the main thread where possible
- [ ] Lazy-load heavy elements (images, math, diagrams) off-screen
- [ ] Large file warning or graceful degradation
- [ ] No data loss on crash (write-ahead or autosave snapshots)
- [ ] Undo history survives document switching within a session
- [ ] Cursor position and scroll position restored on reopen

---

## 24. Extensibility Hooks (Optional)

- [ ] Editor events: onChange, onSave, onCursorMove, onPaste, onKeyDown
- [ ] Register custom commands with hotkeys
- [ ] Register custom markdown renderers for code block languages
- [ ] Register custom slash commands
- [ ] Register custom autocomplete providers
- [ ] Plugin sandboxing (separate from the editor core)

---

## Suggested Build Order

1. Plain text editor with undo, redo, and selection
2. Word and line navigation (Ctrl+Backspace, Ctrl+Arrow, Home, End)
3. Enter continuation for lists, quotes, and tasks, plus Tab and Shift+Tab
4. Auto-pairing and markdown wrap shortcuts
5. Source mode with syntax highlighting
6. Live preview (cursor-aware syntax hiding and inline rendering)
7. Tables, code blocks, and math
8. Links, wikilinks, embeds, and autocomplete
9. Slash commands and command palette
10. Find and replace, outline, and folding
11. Paste handling, images, and drag and drop
12. Themes, modes, and focus features
13. Performance tuning, accessibility, and extensibility
